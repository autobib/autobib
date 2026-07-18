use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::anyhow;

use crate::{
    AsKey, RawEntryData,
    app::{cli::OnConflict, edit::merge_record_data},
    config::Config,
    db::{
        RecordDatabase,
        state::{DatabaseIdResponse, IsEntry, IsMissing, IsVoid, State},
    },
    entry::{BibtexEntry, MutableEntryData, entries_from_bibtex},
    error::{self, RecordError},
    http::Client,
    logger::{error, info, set_failed, warn},
    normalize::{Normalization, Normalize},
    path_hash::AttachmentRoot,
    provider::{IdCandidate, determine_id_candidates, is_canonical},
    record::{
        Alias, Identifier, Key, MappedAliasOrId, MappedKey, RecursiveRemoteResponse,
        get_remote_response_recursive,
    },
};

/// The configuration used to specify the behaviour when importing data.
#[derive(Debug)]
pub struct ImportConfig {
    pub update: Option<OnConflict>,
    pub resolve: bool,
    pub local_fallback: bool,
    pub no_alias: bool,
    pub file_import_root: Option<AttachmentRoot<true>>, // if is_some(), import files to that directory
    pub file_sep: Option<String>,
}

/// Import records from the provided buffer.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn from_buffer<C, W>(
    scratch: &[u8],
    import_config: &ImportConfig,
    record_db: &mut RecordDatabase,
    client: &C,
    config: &Config,
    bibfile: impl std::fmt::Display,
    failed: &mut W,
) -> Result<(), anyhow::Error>
where
    C: Client,
    W: io::Write + ?Sized,
{
    // let mut stdout = stdout_lock_wrap();
    let mut file_import_target = import_config
        .file_import_root
        .as_ref()
        .map(FileImportTarget::new);
    for res in entries_from_bibtex(scratch) {
        match res {
            Ok(entry) => match import_entry(
                entry,
                import_config,
                record_db,
                client,
                config,
                file_import_target.as_mut(),
            )? {
                ImportOutcome::Success => {}
                ImportOutcome::Failure(error, entry) => {
                    writeln!(failed, "% {error}")?;
                    writeln!(failed, "{entry}")?;
                    set_failed();
                }
            },
            Err(err) => {
                error!("Parse error for file '{bibfile}': {err}");
            }
        }
    }

    Ok(())
}

/// The outcome of attempting to import the given entry.
#[must_use]
enum ImportOutcome {
    /// The import was successful.
    Success,
    /// The import failed with an error and with the provided entry.
    Failure(anyhow::Error, BibtexEntry<MutableEntryData>),
}

/// Import a single entry into the record database.
#[inline]
fn import_entry<C>(
    entry: BibtexEntry<MutableEntryData>,
    import_config: &ImportConfig,
    record_db: &mut RecordDatabase,
    client: &C,
    config: &Config,
    file_import_target: Option<&mut FileImportTarget<'_>>,
) -> Result<ImportOutcome, anyhow::Error>
where
    C: Client,
{
    import_entry_impl(
        record_db,
        entry,
        import_config,
        &config.on_insert,
        file_import_target,
        |entry, record_db| {
            let determined = determine_key(entry, config);

            // it is more convenient to do this first since we want to perform
            // the database lookup using the canonical id if possible
            let determined = if import_config.resolve {
                match determined.resolve_reference(client) {
                    Ok(d) => d,
                    Err(err) => return Ok(ImportAction::Fail(err)),
                }
            } else {
                determined
            };

            match determined {
                DeterminedKey::OnlyAlias(alias) => {
                    // we could not determine a canonical identifier
                    if import_config.local_fallback {
                        let id = Identifier::local(&alias);
                        match record_db.state_from_id(&id)? {
                            DatabaseIdResponse::Entry(_, row) => {
                                row.commit()?;
                                Ok(ImportAction::Fail(anyhow!(
                                    "Local id '{id}' already exists.",
                                )))
                            }
                            DatabaseIdResponse::Deleted(_, row) => {
                                row.commit()?;
                                Ok(ImportAction::Fail(anyhow!(
                                    "Local id '{id}' previously existed but was soft-deleted.",
                                )))
                            }
                            DatabaseIdResponse::Void(_, void) => {
                                Ok(ImportAction::Revive(void, id, Some(alias)))
                            }
                            DatabaseIdResponse::Null(null_row) => {
                                Ok(ImportAction::Insert(null_row.delete()?, id, Some(alias)))
                            }
                            DatabaseIdResponse::Unknown(missing) => {
                                Ok(ImportAction::Insert(missing, id, Some(alias)))
                            }
                        }
                    } else {
                        Ok(ImportAction::Fail(anyhow!(
                            "Could not determine candidate key"
                        )))
                    }
                }
                DeterminedKey::Canonical(mkc, maybe_alias) => {
                    match record_db.state_from_id(&mkc.mapped)? {
                        DatabaseIdResponse::Entry(_, state) => Ok(ImportAction::Update(
                            state,
                            import_config.update,
                            mkc.mapped,
                            maybe_alias,
                        )),
                        DatabaseIdResponse::Deleted(_, deleted) => {
                            deleted.commit()?;
                            Ok(ImportAction::Fail(anyhow!(
                                "Identifier '{mkc}' is a deletion marker."
                            )))
                        }
                        DatabaseIdResponse::Void(_, void) => {
                            Ok(ImportAction::Revive(void, mkc.mapped, maybe_alias))
                        }
                        DatabaseIdResponse::Null(null_row) => Ok(ImportAction::Insert(
                            null_row.delete()?,
                            mkc.mapped,
                            maybe_alias,
                        )),
                        DatabaseIdResponse::Unknown(missing) => {
                            Ok(ImportAction::Insert(missing, mkc.mapped, maybe_alias))
                        }
                    }
                }
                DeterminedKey::Reference(mkr, mkc, maybe_alias) => {
                    match record_db.state_from_id(&mkr.mapped)? {
                        DatabaseIdResponse::Entry(data, state) => Ok(ImportAction::Update(
                            state,
                            import_config.update,
                            data.canonical,
                            maybe_alias,
                        )),
                        DatabaseIdResponse::Deleted(_, state) => {
                            state.commit()?;
                            Ok(ImportAction::Fail(anyhow!(
                                "Identifier '{mkr}' is a deletion marker."
                            )))
                        }
                        DatabaseIdResponse::Void(data, state) => {
                            Ok(ImportAction::Revive(state, data.canonical, maybe_alias))
                        }
                        DatabaseIdResponse::Null(state) => match mkc {
                            Some(canonical) => Ok(ImportAction::Insert(
                                state.delete()?,
                                canonical.mapped,
                                maybe_alias,
                            )),
                            None => Ok(ImportAction::Fail(anyhow!(
                                "Failed to determine canonical id; only found reference id {mkr}"
                            ))),
                        },
                        DatabaseIdResponse::Unknown(state) => match mkc {
                            Some(canonical) => {
                                Ok(ImportAction::Insert(state, canonical.mapped, maybe_alias))
                            }
                            None => Ok(ImportAction::Fail(anyhow!(
                                "Failed to determine canonical id; only found reference id {mkr}"
                            ))),
                        },
                    }
                }
                DeterminedKey::Invalid(err) => Ok(ImportAction::Fail(anyhow!(
                    "Could not determine key from entry: {err}",
                ))),
            }
        },
    )
}

/// The action to take for the given entry.
enum ImportAction<'conn> {
    /// The entry already has data corresponding to the provided row; update the row with the
    /// entry.
    Update(
        State<'conn, IsEntry>,
        Option<OnConflict>,
        Identifier,
        Option<Alias>,
    ),
    /// There is no data for the entry; insert data into the database.
    Insert(State<'conn, IsMissing>, Identifier, Option<Alias>),
    /// There is a void marker; revive it with new data.
    Revive(State<'conn, IsVoid>, Identifier, Option<Alias>),
    /// A key could not be determined from the entry.
    Fail(anyhow::Error),
}

/// A helper function to create a new alias, with logging.
fn create_alias_and_commit(
    row: State<'_, IsEntry>,
    id: &str,
    no_alias: bool,
    maybe_alias: Option<Alias>,
) -> Result<(), rusqlite::Error> {
    if !no_alias && let Some(alias) = maybe_alias {
        info!("Creating alias '{alias}' for '{id}'");
        if let Some(other_id) = row.ensure_alias(&alias)? {
            warn!(
                concat!(
                    "Alias '{}' already exists and refers to '{}'. ",
                    "'{}' will be a different record."
                ),
                alias, other_id, id,
            );
        }
    }
    row.commit()?;
    Ok(())
}

/// File import struct with scratch space to reduce re-allocation.
struct FileImportTarget<'a> {
    root: &'a AttachmentRoot<true>,
    path: PathBuf,
}

impl<'a> FileImportTarget<'a> {
    fn new(root: &'a AttachmentRoot<true>) -> Self {
        Self {
            root,
            path: PathBuf::new(),
        }
    }

    /// Import the file at the provided path to the attachment directory of the provided identifier.
    fn import_file(
        &mut self,
        source_path: &Path,
        canonical: &Identifier,
    ) -> Result<(), anyhow::Error> {
        self.root.attachment_dir_in(canonical, &mut self.path);
        match source_path.file_name() {
            None => anyhow::bail!("Cannot import filename containing relative path"),
            Some(file_name) => {
                fs::create_dir_all(&self.path)?;
                self.path.push(file_name);
                // FIXME: this is a TOCTOU error
                if !self.path.exists() {
                    fs::copy(source_path, &self.path)?;
                }
            }
        }
        Ok(())
    }
}

fn normalize_data(
    entry: &mut BibtexEntry<MutableEntryData>,
    nl: &Normalization,
    file_import_target: Option<&mut FileImportTarget<'_>>,
    file_sep: &Option<String>,
    canonical: &Identifier,
) -> Result<(), anyhow::Error> {
    entry.record_data.normalize(nl);
    if let Some(file_import_target) = file_import_target
        && let Some(path) = entry.record_data.remove("file")
    {
        let path_str = path.as_ref();
        if let Some(sep) = file_sep {
            for component in path_str.split(sep) {
                if let Err(err) = file_import_target.import_file(component.as_ref(), canonical) {
                    anyhow::bail!("Failed to import file '{component}': {err}");
                }
            }
        } else if let Err(err) = file_import_target.import_file(path_str.as_ref(), canonical) {
            anyhow::bail!("Failed to import file '{path}': {err}");
        }
    }
    Ok(())
}

/// The actual import implementation, which is generic over the `determine_action` closure which
/// encodes the process of converting an entry into a relevant [`ImportAction`].
#[inline]
fn import_entry_impl<F>(
    record_db: &mut RecordDatabase,
    mut entry: BibtexEntry<MutableEntryData>,
    import_config: &ImportConfig,
    // no_alias: bool,
    nl: &Normalization,
    file_import_target: Option<&mut FileImportTarget<'_>>,
    mut determine_action: F,
) -> Result<ImportOutcome, anyhow::Error>
where
    F: for<'conn> FnMut(
        &BibtexEntry<MutableEntryData>,
        &'conn mut RecordDatabase,
    ) -> Result<ImportAction<'conn>, error::Error>,
{
    match determine_action(&entry, record_db)? {
        ImportAction::Update(row, update_mode, id, maybe_alias) => {
            if let Some(on_conflict) = update_mode {
                if let Err(err) = normalize_data(
                    &mut entry,
                    nl,
                    file_import_target,
                    &import_config.file_sep,
                    &id,
                ) {
                    return Ok(ImportOutcome::Failure(err, entry));
                }

                let current_data = row.get_data()?.data;
                let mut existing_record = MutableEntryData::from_entry_data(&current_data);
                merge_record_data(
                    on_conflict,
                    &mut existing_record,
                    std::iter::once(entry.data()),
                    &id,
                )?;

                let new_data = RawEntryData::from_entry_data(&existing_record);

                info!("Updating data for record with identifier '{id}'");
                let new_row = row.modify(&new_data)?.state;

                create_alias_and_commit(new_row, id.as_key(), import_config.no_alias, maybe_alias)?;
            } else {
                info!("Skipping identifier '{id}': already present in database");
            }
            Ok(ImportOutcome::Success)
        }
        ImportAction::Insert(missing, canonical, maybe_alias) => {
            if let Err(err) = normalize_data(
                &mut entry,
                nl,
                file_import_target,
                &import_config.file_sep,
                &canonical,
            ) {
                return Ok(ImportOutcome::Failure(err, entry));
            }

            info!("Inserting new record with identifier '{canonical}'");
            let row = missing
                .insert_entry_data(&entry.record_data, &canonical)?
                .state;
            create_alias_and_commit(row, canonical.as_key(), import_config.no_alias, maybe_alias)?;
            Ok(ImportOutcome::Success)
        }
        ImportAction::Revive(void, id, maybe_alias) => {
            if let Err(err) = normalize_data(
                &mut entry,
                nl,
                file_import_target,
                &import_config.file_sep,
                &id,
            ) {
                return Ok(ImportOutcome::Failure(err, entry));
            }

            info!("Re-inserting record with canonical id '{id}'");
            let row = void
                .reinsert(&RawEntryData::from_entry_data(&entry.record_data))?
                .state;
            create_alias_and_commit(row, id.as_key(), import_config.no_alias, maybe_alias)?;
            Ok(ImportOutcome::Success)
        }
        ImportAction::Fail(prompt) => Ok(ImportOutcome::Failure(prompt, entry)),
    }
}

pub enum DeterminedKey {
    /// The optimal identifier found was canonical.
    Canonical(MappedKey, Option<Alias>),
    /// The optimal identifier found was a reference identifier, with a sub-optimal canonical
    /// fallback.
    Reference(MappedKey, Option<MappedKey>, Option<Alias>),
    /// No remote identifier could be determined, but the citation key was an alias.
    OnlyAlias(Alias),
    /// No identifier could be determined.
    Invalid(RecordError),
}

impl DeterminedKey {
    /// Convert a 'reference' variant into a 'canonical' variant, returning an error if this fails.
    pub fn resolve_reference<C: Client>(self, client: &C) -> Result<Self, anyhow::Error> {
        if let Self::Reference(mkr, _, maybe_alias) = self {
            let MappedKey { mapped, original } = mkr;
            match get_remote_response_recursive(mapped, client)? {
                RecursiveRemoteResponse::Exists(_, id) => Ok(Self::Canonical(
                    MappedKey {
                        mapped: id,
                        original,
                    },
                    maybe_alias,
                )),
                RecursiveRemoteResponse::Null(id) => {
                    Err(anyhow!("Determined reference key '{id}' is null"))
                }
            }
        } else {
            Ok(self)
        }
    }
}

/// Determine the key associated with the provided entry.
pub fn determine_key(entry: &BibtexEntry<MutableEntryData>, config: &Config) -> DeterminedKey {
    let score_fn = |id: &Identifier| config.score_key(id.as_key());

    let resolved = Key::from(entry.key.as_ref())
        .resolve(&config.alias_transform)
        .map(Into::into);
    match resolved {
        // if it is an alias, just get the best key from the data
        Ok(MappedAliasOrId::Alias(alias)) => {
            match determine_id_candidates(entry.data(), score_fn, None, None) {
                IdCandidate::OptimalCanonical(mkc) => DeterminedKey::Canonical(mkc, Some(alias)),
                IdCandidate::OptimalReference(mkc, mkr) => {
                    DeterminedKey::Reference(mkc, mkr, Some(alias))
                }
                IdCandidate::None => DeterminedKey::OnlyAlias(alias),
            }
        }
        // if it is an error, check if the data returned something, or return an error
        Err(err) => match determine_id_candidates(entry.data(), score_fn, None, None) {
            IdCandidate::OptimalCanonical(mapped_key) => DeterminedKey::Canonical(mapped_key, None),
            IdCandidate::OptimalReference(mkr, mkc) => DeterminedKey::Reference(mkr, mkc, None),
            IdCandidate::None => DeterminedKey::Invalid(err),
        },
        // otherwise, see if we can find a better key in the data
        Ok(MappedAliasOrId::Id(id_from_key)) => {
            let best_keypair = if is_canonical(id_from_key.mapped.provider()) {
                determine_id_candidates(entry.data(), score_fn, Some(id_from_key), None)
            } else {
                determine_id_candidates(entry.data(), score_fn, None, Some(id_from_key))
            };

            match best_keypair {
                IdCandidate::OptimalCanonical(mkc) => DeterminedKey::Canonical(mkc, None),
                IdCandidate::OptimalReference(mkc, mkr) => DeterminedKey::Reference(mkc, mkr, None),
                // unreachable since we started with a candidate
                IdCandidate::None => unreachable!(),
            }
        }
    }
}
