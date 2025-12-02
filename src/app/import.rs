use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::anyhow;

use crate::{
    Identifier, RawEntryData,
    app::{cli::OnConflict, edit::merge_record_data},
    config::Config,
    db::{
        RecordDatabase,
        state::{IsEntry, IsMissing, IsVoid, RemoteIdState, State},
    },
    entry::{Entry, MutableEntryData, entries_from_bibtex},
    error::{self, RecordError},
    http::Client,
    logger::{error, info, set_failed, warn},
    normalize::{Normalization, Normalize},
    output::stdout_lock_wrap,
    path_hash::PathHash,
    provider::{RemoteIdCandidate, determine_remote_id_candidates, is_canonical},
    record::{
        Alias, MappedAliasOrRemoteId, MappedKey, RecordId, RecursiveRemoteResponse, RemoteId,
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
    pub include_files: bool,
    pub file_sep: Option<String>,
}

/// Import records from the provided buffer.
#[inline]
pub fn from_buffer<F, C>(
    scratch: &[u8],
    import_config: &ImportConfig,
    record_db: &mut RecordDatabase,
    client: &C,
    config: &Config<F>,
    attachment_root: &Path,
    bibfile: impl std::fmt::Display,
) -> Result<(), anyhow::Error>
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
{
    let mut attachment_root_buf = if import_config.include_files {
        Some(PathBuf::new())
    } else {
        None
    };
    let mut stdout = stdout_lock_wrap();
    for res in entries_from_bibtex(scratch) {
        if let Some(p) = attachment_root_buf.as_mut() {
            p.clear();
            p.push(attachment_root);
        };
        match res {
            Ok(entry) => match import_entry(
                entry,
                import_config,
                record_db,
                client,
                config,
                attachment_root_buf.as_mut(),
            )? {
                ImportOutcome::Success => {}
                ImportOutcome::Failure(error, entry) => {
                    writeln!(&mut stdout, "% {error}")?;
                    writeln!(&mut stdout, "{entry}")?;
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
    Failure(anyhow::Error, Entry<MutableEntryData>),
}

/// Import a single entry into the record database.
#[inline]
fn import_entry<F, C>(
    entry: Entry<MutableEntryData>,
    import_config: &ImportConfig,
    record_db: &mut RecordDatabase,
    client: &C,
    config: &Config<F>,
    attachment_root: Option<&mut PathBuf>,
) -> Result<ImportOutcome, anyhow::Error>
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
{
    import_entry_impl(
        record_db,
        entry,
        import_config,
        &config.on_insert,
        attachment_root,
        |entry, record_db| {
            let determined = determine_key::<F>(entry, config);

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
                        let remote_id = RemoteId::local(&alias);
                        match record_db.state_from_remote_id(&remote_id)? {
                            RemoteIdState::Entry(_, row) => {
                                row.commit()?;
                                Ok(ImportAction::Fail(anyhow!(
                                    "Local id '{remote_id}' already exists.",
                                )))
                            }
                            RemoteIdState::Deleted(_, row) => {
                                row.commit()?;
                                Ok(ImportAction::Fail(anyhow!(
                                    "Local id '{remote_id}' previously existed but was soft-deleted.",
                                )))
                            }
                            RemoteIdState::Void(_, void) => {
                                Ok(ImportAction::Revive(void, remote_id, Some(alias)))
                            }
                            RemoteIdState::Null(null_row) => Ok(ImportAction::Insert(
                                null_row.delete()?,
                                remote_id,
                                Some(alias),
                            )),
                            RemoteIdState::Unknown(missing) => {
                                Ok(ImportAction::Insert(missing, remote_id, Some(alias)))
                            }
                        }
                    } else {
                        Ok(ImportAction::Fail(anyhow!(
                            "Could not determine candidate key"
                        )))
                    }
                }
                DeterminedKey::Canonical(mkc, maybe_alias) => {
                    match record_db.state_from_remote_id(&mkc.mapped)? {
                        RemoteIdState::Entry(_, state) => Ok(ImportAction::Update(
                            state,
                            import_config.update,
                            mkc.mapped,
                            maybe_alias,
                        )),
                        RemoteIdState::Deleted(_, deleted) => {
                            deleted.commit()?;
                            Ok(ImportAction::Fail(anyhow!(
                                "Identifier '{mkc}' is a deletion marker."
                            )))
                        }
                        RemoteIdState::Void(_, void) => {
                            Ok(ImportAction::Revive(void, mkc.mapped, maybe_alias))
                        }
                        RemoteIdState::Null(null_row) => Ok(ImportAction::Insert(
                            null_row.delete()?,
                            mkc.mapped,
                            maybe_alias,
                        )),
                        RemoteIdState::Unknown(missing) => {
                            Ok(ImportAction::Insert(missing, mkc.mapped, maybe_alias))
                        }
                    }
                }
                DeterminedKey::Reference(mkr, mkc, maybe_alias) => {
                    match record_db.state_from_remote_id(&mkr.mapped)? {
                        RemoteIdState::Entry(data, state) => Ok(ImportAction::Update(
                            state,
                            import_config.update,
                            data.canonical,
                            maybe_alias,
                        )),
                        RemoteIdState::Deleted(_, state) => {
                            state.commit()?;
                            Ok(ImportAction::Fail(anyhow!(
                                "Identifier '{mkr}' is a deletion marker."
                            )))
                        }
                        RemoteIdState::Void(data, state) => {
                            Ok(ImportAction::Revive(state, data.canonical, maybe_alias))
                        }
                        RemoteIdState::Null(state) => match mkc {
                            Some(canonical) => Ok(ImportAction::Insert(
                                state.delete()?,
                                canonical.mapped,
                                maybe_alias,
                            )),
                            None => Ok(ImportAction::Fail(anyhow!(
                                "Failed to determine canonical id; only found reference id {mkr}"
                            ))),
                        },
                        RemoteIdState::Unknown(state) => match mkc {
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
        RemoteId,
        Option<Alias>,
    ),
    /// There is no data for the entry; insert data into the database.
    Insert(State<'conn, IsMissing>, RemoteId, Option<Alias>),
    /// There is a void marker; revive it with new data.
    Revive(State<'conn, IsVoid>, RemoteId, Option<Alias>),
    /// A key could not be determined from the entry.
    Fail(anyhow::Error),
}

/// A helper function to create a new alias, with logging.
fn create_alias_and_commit(
    row: State<'_, IsEntry>,
    remote_id: &str,
    no_alias: bool,
    maybe_alias: Option<Alias>,
) -> Result<(), rusqlite::Error> {
    if !no_alias && let Some(alias) = maybe_alias {
        info!("Creating alias '{alias}' for '{remote_id}'");
        if let Some(other_remote_id) = row.ensure_alias(&alias)? {
            warn!(
                concat!(
                    "Alias '{}' already exists and refers to '{}'. ",
                    "'{}' will be a different record."
                ),
                alias, other_remote_id, remote_id,
            );
        }
    }
    row.commit()?;
    Ok(())
}

fn import_file(
    source_path: &Path,
    target_path: &mut PathBuf,
    canonical: &RemoteId,
) -> Result<(), anyhow::Error> {
    canonical.extend_attachments_path(target_path);
    match source_path.file_name() {
        None => anyhow::bail!("Cannot import filename containing relative path"),
        Some(file_name) => {
            fs::create_dir_all(&target_path)?;
            target_path.push(file_name);
            // FIXME: this is a TOCTOU error
            if !target_path.exists() {
                fs::copy(source_path, target_path)?;
            }
        }
    }
    Ok(())
}

fn normalize_data(
    entry: &mut Entry<MutableEntryData>,
    nl: &Normalization,
    include_files: Option<&mut PathBuf>,
    file_sep: &Option<String>,
    canonical: &RemoteId,
) -> Result<(), anyhow::Error> {
    entry.record_data.normalize(nl);
    if let Some(target_path) = include_files
        && let Some(path) = entry.record_data.remove("file")
    {
        let path_str = path.as_ref();
        if let Some(sep) = file_sep {
            for component in path_str.split(sep) {
                if let Err(err) = import_file(component.as_ref(), target_path, canonical) {
                    anyhow::bail!("Failed to import file '{component}': {err}");
                }
            }
        } else if let Err(err) = import_file(path_str.as_ref(), target_path, canonical) {
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
    mut entry: Entry<MutableEntryData>,
    import_config: &ImportConfig,
    // no_alias: bool,
    nl: &Normalization,
    attachment_root: Option<&mut PathBuf>,
    mut determine_action: F,
) -> Result<ImportOutcome, anyhow::Error>
where
    F: for<'conn> FnMut(
        &Entry<MutableEntryData>,
        &'conn mut RecordDatabase,
    ) -> Result<ImportAction<'conn>, error::Error>,
{
    match determine_action(&entry, record_db)? {
        ImportAction::Update(row, update_mode, remote_id, maybe_alias) => {
            if let Some(on_conflict) = update_mode {
                if let Err(err) = normalize_data(
                    &mut entry,
                    nl,
                    attachment_root,
                    &import_config.file_sep,
                    &remote_id,
                ) {
                    return Ok(ImportOutcome::Failure(err, entry));
                }

                let current_data = row.get_data()?.data;
                let mut existing_record = MutableEntryData::from_entry_data(&current_data);
                merge_record_data(
                    on_conflict,
                    &mut existing_record,
                    std::iter::once(entry.data()),
                    &remote_id,
                )?;

                let new_data = RawEntryData::from_entry_data(&existing_record);

                info!("Updating data for record with identifier '{remote_id}'");
                let new_row = row.modify(&new_data)?;

                create_alias_and_commit(
                    new_row,
                    remote_id.name(),
                    import_config.no_alias,
                    maybe_alias,
                )?;
            } else {
                info!("Skipping identifier '{remote_id}': already present in database");
            }
            Ok(ImportOutcome::Success)
        }
        ImportAction::Insert(missing, canonical, maybe_alias) => {
            if let Err(err) = normalize_data(
                &mut entry,
                nl,
                attachment_root,
                &import_config.file_sep,
                &canonical,
            ) {
                return Ok(ImportOutcome::Failure(err, entry));
            }

            info!("Inserting new record with identifier '{canonical}'");
            let row = missing.insert_entry_data(&entry.record_data, &canonical)?;
            create_alias_and_commit(row, canonical.name(), import_config.no_alias, maybe_alias)?;
            Ok(ImportOutcome::Success)
        }
        ImportAction::Revive(void, remote_id, maybe_alias) => {
            if let Err(err) = normalize_data(
                &mut entry,
                nl,
                attachment_root,
                &import_config.file_sep,
                &remote_id,
            ) {
                return Ok(ImportOutcome::Failure(err, entry));
            }

            info!("Re-inserting record with canonical id '{remote_id}'");
            let row = void.reinsert(&RawEntryData::from_entry_data(&entry.record_data))?;
            create_alias_and_commit(row, remote_id.name(), import_config.no_alias, maybe_alias)?;
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
                RecursiveRemoteResponse::Exists(_, remote_id) => Ok(Self::Canonical(
                    MappedKey {
                        mapped: remote_id,
                        original,
                    },
                    maybe_alias,
                )),
                RecursiveRemoteResponse::Null(remote_id) => {
                    Err(anyhow!("Determined reference key '{remote_id}' is null"))
                }
            }
        } else {
            Ok(self)
        }
    }
}

/// Determine the key associated with the provided entry.
pub fn determine_key<F>(entry: &Entry<MutableEntryData>, config: &Config<F>) -> DeterminedKey
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
{
    let score_fn = |id: &RemoteId| {
        std::cmp::Reverse(
            config
                .preferred_providers
                .iter()
                .position(|pref| pref == id.provider())
                .unwrap_or(config.preferred_providers.len()),
        )
    };

    // let from_data = determine_remote_id_candidates(entry.data(), score_fn);

    let resolved = RecordId::from(entry.key.as_ref())
        .resolve(&config.alias_transform)
        .map(Into::into);
    match resolved {
        // if it is an alias, just get the best key from the data
        Ok(MappedAliasOrRemoteId::Alias(alias)) => {
            match determine_remote_id_candidates(entry.data(), score_fn, None, None) {
                RemoteIdCandidate::OptimalCanonical(mkc) => {
                    DeterminedKey::Canonical(mkc, Some(alias))
                }
                RemoteIdCandidate::OptimalReference(mkc, mkr) => {
                    DeterminedKey::Reference(mkc, mkr, Some(alias))
                }
                RemoteIdCandidate::None => DeterminedKey::OnlyAlias(alias),
            }
        }
        // if it is an error, check if the data returned something, or return an error
        Err(err) => match determine_remote_id_candidates(entry.data(), score_fn, None, None) {
            RemoteIdCandidate::OptimalCanonical(mapped_key) => {
                DeterminedKey::Canonical(mapped_key, None)
            }
            RemoteIdCandidate::OptimalReference(mkr, mkc) => {
                DeterminedKey::Reference(mkr, mkc, None)
            }
            RemoteIdCandidate::None => DeterminedKey::Invalid(err),
        },
        // otherwise, see if we can find a better key in the data
        Ok(MappedAliasOrRemoteId::RemoteId(id_from_key)) => {
            let best_keypair = if is_canonical(id_from_key.mapped.provider()) {
                determine_remote_id_candidates(entry.data(), score_fn, Some(id_from_key), None)
            } else {
                determine_remote_id_candidates(entry.data(), score_fn, None, Some(id_from_key))
            };

            match best_keypair {
                RemoteIdCandidate::OptimalCanonical(mkc) => DeterminedKey::Canonical(mkc, None),
                RemoteIdCandidate::OptimalReference(mkc, mkr) => {
                    DeterminedKey::Reference(mkc, mkr, None)
                }
                // unreachable since we started with a candidate
                RemoteIdCandidate::None => unreachable!(),
            }
        }
    }
}
