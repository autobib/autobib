use std::str::FromStr;

use anyhow::anyhow;

use crate::{
    CitationKey,
    app::{
        cli::{ImportMode, OnConflict},
        edit::merge_record_data,
    },
    config::Config,
    db::{
        RecordDatabase,
        state::{Missing, RecordRow, RemoteIdState, State},
    },
    entry::{Entry, EntryKey, MutableEntryData, entries_from_bibtex},
    error::{self, RecordError},
    http::Client,
    logger::{error, info, set_failed, warn},
    normalize::{Normalization, Normalize},
    output::owriteln,
    provider::{determine_remote_id_candidates, is_canonical},
    record::{
        Alias, MappedAliasOrRemoteId, MappedKey, RecordId, RemoteId, RemoteRecordRowResponse,
        get_record_row_remote,
    },
    term::{Confirm, Editor, EditorConfig},
};

/// The configuration used to specify the behaviour when importing data.
#[derive(Debug)]
pub struct ImportConfig {
    pub on_conflict: OnConflict,
    pub import_mode: ImportMode,
    pub no_alias: bool,
    pub no_interactive: bool,
    pub replace_colons: Option<EntryKey<String>>,
    pub log_failures: bool,
}

/// Import records from the provided buffer.
#[inline]
pub fn from_buffer<F, C>(
    scratch: &[u8],
    import_config: &ImportConfig,
    record_db: &mut RecordDatabase,
    client: &C,
    config: &Config<F>,
    bibfile: impl std::fmt::Display,
) -> Result<(), anyhow::Error>
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
{
    for res in entries_from_bibtex(scratch) {
        match res {
            Ok(mut entry) => {
                // replace colons with the replacement value, if a replacement
                // value is passed and a substitution occurs
                if let Some(ref s) = import_config.replace_colons
                    && let Some(replacement) = entry.key.substitute(':', s)
                {
                    entry.key = replacement;
                }

                match import_entry(entry, import_config, record_db, client, config)? {
                    ImportOutcome::Success => {}
                    ImportOutcome::Failure(error, entry) => {
                        if import_config.log_failures {
                            owriteln!("% Import failed: {error}")?;
                            owriteln!("{entry}")?;
                            set_failed();
                        } else {
                            error!(
                                "Failed to import entry from file '{bibfile}' with key '{}'",
                                entry.key().as_ref()
                            );
                        }
                    }
                    ImportOutcome::UserCancelled => {
                        error!("Cancelled editing; entry was not imported!");
                    }
                }
            }
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
    /// There was an error while importing the entry, which the user did not fix.
    UserCancelled,
}

/// Import a single entry into the record database.
#[inline]
fn import_entry<F, C>(
    entry: Entry<MutableEntryData>,
    import_config: &ImportConfig,
    record_db: &mut RecordDatabase,
    client: &C,
    config: &Config<F>,
) -> Result<ImportOutcome, anyhow::Error>
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
{
    match import_config.import_mode {
        ImportMode::Local => import_entry_impl(
            record_db,
            entry,
            import_config.no_interactive,
            import_config.no_alias,
            &config.on_insert,
            |entry, record_db| {
                let alias = match Alias::from_str(entry.key.as_ref()) {
                    Ok(alias) => alias,
                    Err(alias_conversion_error) => {
                        return Ok(ImportAction::PromptNewKey(anyhow!(alias_conversion_error,)));
                    }
                };
                handle_local_alias(alias, record_db)
            },
        ),
        ImportMode::DetermineKey => import_entry_impl(
            record_db,
            entry,
            import_config.no_interactive,
            import_config.no_alias,
            &config.on_insert,
            |entry, record_db| {
                // we require a canonical identifier since we do not perform any remote resolution
                match determine_key::<F, C>(entry, true, config) {
                    DeterminedKey::Alias(alias) => {
                        // we could not determine a remote identifier, so we just fall back to the
                        // 'local' behaviour
                        handle_local_alias(alias, record_db)
                    }
                    DeterminedKey::RemoteId(mapped_key, maybe_alias) => {
                        match record_db.state_from_remote_id(&mapped_key.mapped)? {
                            RemoteIdState::Existent(row) => Ok(ImportAction::Update(
                                row,
                                import_config.on_conflict,
                                mapped_key.to_string(),
                                maybe_alias,
                            )),
                            RemoteIdState::Null(null_row) => Ok(ImportAction::Insert(
                                null_row.delete()?,
                                mapped_key.mapped,
                                maybe_alias,
                            )),
                            RemoteIdState::Unknown(missing) => Ok(ImportAction::Insert(
                                missing,
                                mapped_key.mapped,
                                maybe_alias,
                            )),
                        }
                    }
                    DeterminedKey::NotCanonical(mapped_key) => {
                        Ok(ImportAction::PromptNewKey(anyhow!(
                            concat!(
                                "Could not determined canonical identifier: ",
                                "found reference identifier '{}'"
                            ),
                            mapped_key
                        )))
                    }
                    DeterminedKey::Invalid(err) => Ok(ImportAction::PromptNewKey(anyhow!(
                        "Could not determine key from entry: {err}",
                    ))),
                }
            },
        ),
        ImportMode::Retrieve => import_entry_retrieve_impl(
            record_db,
            entry,
            import_config,
            config,
            client,
            |alias, record_db| {
                let remote_id = RemoteId::local(&alias);
                match record_db.state_from_remote_id(&remote_id)? {
                    RemoteIdState::Existent(row) => {
                        row.commit()?;
                        Ok(ImportAction::PromptNewKey(anyhow!(
                            "Local id '{remote_id}' already exists.",
                        )))
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
            },
        ),
        ImportMode::RetrieveOnly => import_entry_retrieve_impl(
            record_db,
            entry,
            import_config,
            config,
            client,
            |alias, _| {
                Ok(ImportAction::PromptNewKey(anyhow!(
                    "Could not determine remote identifier from entry with key {alias}",
                )))
            },
        ),
    }
}

/// The action to take for the given entry.
enum ImportAction<'conn> {
    /// The entry already has data corresponding to the provided row; update the row with the
    /// entry.
    Update(State<'conn, RecordRow>, OnConflict, String, Option<Alias>),
    /// There is no data for the entry; data into the database.
    Insert(State<'conn, Missing>, RemoteId, Option<Alias>),
    /// A key could not be determined from the entry; prompt for a new key (if interactive).
    PromptNewKey(anyhow::Error),
}

/// A helper function to create a new alias, with logging.
fn create_alias(
    row: State<'_, RecordRow>,
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

/// The actual import implementation, which is generic over the `determine_action` closure which
/// encodes the process of converting an entry into a relevant [`ImportAction`].
#[inline]
fn import_entry_impl<F>(
    record_db: &mut RecordDatabase,
    mut entry: Entry<MutableEntryData>,
    no_interactive: bool,
    no_alias: bool,
    nl: &Normalization,
    mut determine_action: F,
) -> Result<ImportOutcome, anyhow::Error>
where
    F: for<'conn> FnMut(
        &Entry<MutableEntryData>,
        &'conn mut RecordDatabase,
    ) -> Result<ImportAction<'conn>, error::Error>,
{
    loop {
        match determine_action(&entry, record_db)? {
            ImportAction::Update(row, update_mode, remote_id, maybe_alias) => {
                entry.record_data.normalize(nl);
                let raw_record_data = row.get_data()?.data;
                let mut existing_record = MutableEntryData::from_entry_data(&raw_record_data);
                merge_record_data(
                    update_mode,
                    &mut existing_record,
                    std::iter::once(entry.data()),
                    &remote_id,
                )?;
                row.save_to_changelog()?;
                row.update_entry_data(&existing_record)?;
                create_alias(row, &remote_id, no_alias, maybe_alias)?;
                return Ok(ImportOutcome::Success);
            }
            ImportAction::Insert(missing, remote_id, maybe_alias) => {
                info!("Inserting new record with identifier '{remote_id}'");
                entry.record_data.normalize(nl);
                let row = missing.insert_entry_data(&entry.record_data, &remote_id)?;
                create_alias(row, remote_id.name(), no_alias, maybe_alias)?;
                return Ok(ImportOutcome::Success);
            }
            ImportAction::PromptNewKey(prompt) => {
                if no_interactive {
                    return Ok(ImportOutcome::Failure(prompt, entry));
                } else {
                    warn!("Failed to determine key: {prompt}");
                    if Confirm::new("Edit entry and try again?", true).confirm()? {
                        match Editor::new(EditorConfig { suffix: ".bib" }).edit(&entry)? {
                            Some(new_entry) => entry = new_entry,
                            None => return Ok(ImportOutcome::UserCancelled),
                        }
                    } else {
                        return Ok(ImportOutcome::UserCancelled);
                    }
                }
            }
        }
    }
}

/// This method handles the implementation for the `--retrieve` and `--retrieve-only` flags. In
/// both cases, if there is a remote identifier, we automatically attempt to retrieve the data for
/// that identifier first.
///
/// The only difference in behaviour in the two cases is what should be done when the identifier
/// is an alias, and no remote identifier could be determined from the entry: in this case, the
/// behaviour is handled by the `handle_alias` closure.
#[inline]
fn import_entry_retrieve_impl<A, F, C>(
    record_db: &mut RecordDatabase,
    entry: Entry<MutableEntryData>,
    import_config: &ImportConfig,
    config: &Config<F>,
    client: &C,
    mut handle_alias: A,
) -> Result<ImportOutcome, anyhow::Error>
where
    A: for<'conn> FnMut(
        Alias,
        &'conn mut RecordDatabase,
    ) -> Result<ImportAction<'conn>, error::Error>,
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
{
    import_entry_impl(
        record_db,
        entry,
        import_config.no_interactive,
        import_config.no_alias,
        &config.on_insert,
        // we do not require a canonical identifier since we perform remote resolution
        |entry, record_db| match determine_key::<F, C>(entry, false, config) {
            DeterminedKey::Alias(alias) => handle_alias(alias, record_db),
            DeterminedKey::RemoteId(mapped_key, maybe_alias) => {
                let remote_id = mapped_key.mapped;
                match get_record_row_remote(record_db, remote_id, client, config)? {
                    RemoteRecordRowResponse::Exists(record, row) => Ok(ImportAction::Update(
                        row,
                        import_config.on_conflict,
                        record.key,
                        maybe_alias,
                    )),
                    RemoteRecordRowResponse::Null(remote_id, null_row) => Ok(ImportAction::Insert(
                        null_row.delete()?,
                        remote_id,
                        maybe_alias,
                    )),
                }
            }
            DeterminedKey::NotCanonical(mapped_key) => Ok(ImportAction::PromptNewKey(anyhow!(
                concat!(
                    "Could not determined canonical identifier: ",
                    "found reference identifier '{}'"
                ),
                mapped_key
            ))),
            DeterminedKey::Invalid(err) => Ok(ImportAction::PromptNewKey(anyhow!(
                "Could not determine key from entry: {err}",
            ))),
        },
    )
}

/// Process an alias in an import scheme which creates a local record.
fn handle_local_alias(
    alias: Alias,
    record_db: &mut RecordDatabase,
) -> Result<ImportAction<'_>, error::Error> {
    let remote_id = RemoteId::local(&alias);
    match record_db.state_from_remote_id(&remote_id)? {
        RemoteIdState::Existent(row) => {
            row.commit()?;
            Ok(ImportAction::PromptNewKey(anyhow!(
                "Local id '{remote_id}' already exists.",
            )))
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
}

/// The outcome of attempting to determine a canonical identifier associated with an entry.
enum DeterminedKey {
    /// The entry key was an alias and no remote identifier could be determined.
    Alias(Alias),
    /// A remote identifier was determined, and the entry key was possibly a valid alias.
    RemoteId(MappedKey, Option<Alias>),
    /// A remote identifier was found, but it was not canonical and a canonical identifier was
    /// requested.
    NotCanonical(MappedKey),
    /// No identifier could be found; the entry has an invalid key.
    Invalid(RecordError),
}

/// Determine the key associated with the provided entry.
#[inline]
fn determine_key<F, C>(
    entry: &Entry<MutableEntryData>,
    require_canonical: bool,
    config: &Config<F>,
) -> DeterminedKey
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
{
    let resolved = RecordId::from(entry.key.as_ref())
        .resolve(&config.alias_transform)
        .map(Into::into);
    match resolved {
        Ok(MappedAliasOrRemoteId::Alias(alias)) => {
            match best_key_from_data::<F, C>(entry.data(), require_canonical, config) {
                Some(remote_id) => DeterminedKey::RemoteId(remote_id, Some(alias)),
                None => DeterminedKey::Alias(alias),
            }
        }
        Ok(MappedAliasOrRemoteId::RemoteId(mapped_key)) => {
            if !require_canonical || is_canonical::<C>(mapped_key.mapped.provider()) {
                DeterminedKey::RemoteId(mapped_key, None)
            } else {
                match best_key_from_data::<F, C>(entry.data(), require_canonical, config) {
                    Some(mapped_key) => DeterminedKey::RemoteId(mapped_key, None),
                    None => DeterminedKey::NotCanonical(mapped_key),
                }
            }
        }
        Err(err) => match best_key_from_data::<F, C>(entry.data(), require_canonical, config) {
            Some(mapped_key) => DeterminedKey::RemoteId(mapped_key, None),
            None => DeterminedKey::Invalid(err),
        },
    }
}

/// Determine the 'best' key from the data: that is, the key which matches a preferred
/// provider with the smallest possible index.
#[inline]
fn best_key_from_data<F, C>(
    data: &MutableEntryData,
    require_canonical: bool,
    config: &Config<F>,
) -> Option<MappedKey>
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
{
    let mut highest_scoring_candidate: Option<(MappedKey, usize)> = None;
    determine_remote_id_candidates(
        data,
        |provider| {
            if require_canonical && !is_canonical::<C>(provider) {
                None
            } else {
                config
                    .preferred_providers
                    .iter()
                    .position(|pref| pref == provider)
            }
        },
        |remote_id, score| match highest_scoring_candidate.as_mut() {
            Some(inner) => {
                if score < inner.1 {
                    *inner = (remote_id, score);
                }
            }
            None => highest_scoring_candidate = Some((remote_id, score)),
        },
    );
    highest_scoring_candidate.map(|(r, _)| r)
}
