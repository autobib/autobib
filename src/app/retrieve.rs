use std::collections::{
    BTreeMap,
    btree_map::Entry::{Occupied, Vacant},
};

use nonempty::NonEmpty;

use crate::{
    config::Config,
    db::{
        RecordDatabase,
        state::{EntryOrDeletedRow, EntryRow, EntryRowData, RecordIdState, State},
    },
    entry::{Entry, EntryKey, RawEntryData},
    error::Error,
    http::Client,
    logger::{error, reraise, suggest},
    record::{Record, RecordId, RecordRowResponse, RemoteId, get_record_row},
};

/// Group valid entries by their canonical id in order to catch duplicate entries.
fn group_valid_entries_by_canonical<T>(
    valid_entries: T,
) -> BTreeMap<RemoteId, NonEmpty<Entry<RawEntryData>>>
where
    T: IntoIterator<Item = (Entry<RawEntryData>, RemoteId)>,
{
    let mut grouped_entries: BTreeMap<RemoteId, NonEmpty<Entry<RawEntryData>>> = BTreeMap::new();
    for (bibtex_entry, canonical) in valid_entries {
        match grouped_entries.entry(canonical) {
            Occupied(e) => e.into_mut().push(bibtex_entry),
            Vacant(e) => {
                e.insert(NonEmpty::singleton(bibtex_entry));
            }
        }
    }
    grouped_entries
}

/// Retrieve and validate BibTeX entries.
pub fn retrieve_and_validate_entries<
    T: Iterator<Item = RecordId>,
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
>(
    citation_keys: T,
    record_db: &mut RecordDatabase,
    client: &C,
    retrieve_only: bool,
    ignore_null: bool,
    config: &Config<F>,
) -> BTreeMap<RemoteId, NonEmpty<Entry<RawEntryData>>> {
    let valid_entries = citation_keys.filter_map(|citation_key| {
        retrieve_and_validate_single_entry(
            record_db,
            citation_key,
            client,
            retrieve_only,
            ignore_null,
            config,
        )
        .unwrap_or_else(|error| {
            reraise(&error);
            None
        })
    });
    group_valid_entries_by_canonical(valid_entries)
}

pub fn retrieve_entries_read_only<
    T: Iterator<Item = RecordId>,
    F: FnOnce() -> Vec<(regex::Regex, String)>,
>(
    citation_keys: T,
    record_db: &mut RecordDatabase,
    retrieve_only: bool,
    ignore_null: bool,
    config: &Config<F>,
) -> BTreeMap<RemoteId, NonEmpty<Entry<RawEntryData>>> {
    let valid_entries = citation_keys.filter_map(|record_id| {
        retrieve_single_entry_read_only(record_db, record_id, retrieve_only, ignore_null, config)
            .unwrap_or_else(|error| {
                error!("{error}");
                None
            })
    });
    group_valid_entries_by_canonical(valid_entries)
}

/// Retrieve a single BibTeX entry if it exists in the database, returning if it does not `Ok(None)` otherwise.
fn retrieve_single_entry_read_only<F: FnOnce() -> Vec<(regex::Regex, String)>>(
    record_db: &mut RecordDatabase,
    citation_key: RecordId,
    retrieve_only: bool,
    ignore_null: bool,
    config: &Config<F>,
) -> Result<Option<(Entry<RawEntryData>, RemoteId)>, Error> {
    match record_db.state_from_record_id(citation_key, &config.alias_transform)? {
        RecordIdState::Entry(key, row) => {
            if retrieve_only {
                row.commit()?;
                Ok(None)
            } else {
                match row.resolve()? {
                    EntryOrDeletedRow::Exists(
                        EntryRowData {
                            data, canonical, ..
                        },
                        state,
                    ) => {
                        let entry = validate_bibtex_key(key, &state)
                            .map(|key| (Entry::new(key, data), canonical));
                        state.commit()?;
                        Ok(entry)
                    }
                    EntryOrDeletedRow::Deleted(deleted_row_data, state) => todo!(),
                }
            }
        }
        RecordIdState::NullRemoteId(remote_id, missing) => {
            if !ignore_null {
                error!("Null record: '{remote_id}'");
            }
            missing.commit()?;
            Ok(None)
        }
        RecordIdState::UndefinedAlias(alias) => {
            if !ignore_null {
                error!("Undefined alias: '{alias}'");
            }
            Ok(None)
        }
        RecordIdState::InvalidRemoteId(err) => {
            reraise(&err);
            Ok(None)
        }
        RecordIdState::Unknown(unknown) => {
            let mapped = unknown.combine_and_commit()?;
            error!("Database does not contain key: {mapped}");
            Ok(None)
        }
    }
}

/// Retrieve and validate a single BibTeX entry.
fn retrieve_and_validate_single_entry<F, C>(
    record_db: &mut RecordDatabase,
    citation_key: RecordId,
    client: &C,
    retrieve_only: bool,
    ignore_null: bool,
    config: &Config<F>,
) -> Result<Option<(Entry<RawEntryData>, RemoteId)>, Error>
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
{
    match get_record_row(record_db, citation_key, client, config)? {
        RecordRowResponse::Exists(record, row) => {
            if retrieve_only {
                row.commit()?;
                Ok(None)
            } else {
                let Record {
                    key,
                    data,
                    canonical,
                } = record;
                let entry =
                    validate_bibtex_key(key, &row).map(|key| (Entry::new(key, data), canonical));
                row.commit()?;
                Ok(entry)
            }
        }
        RecordRowResponse::Deleted(data, row) => {
            if !ignore_null {
                match data.replacement {
                    Some(repl) => error!("Record '{}' replaced with '{repl}'.", data.key),
                    None => error!("Record '{}' was deleted.", data.key),
                }
            }
            row.commit()?;
            Ok(None)
        }
        RecordRowResponse::NullRemoteId(remote_id, missing) => {
            if !ignore_null {
                error!("Null record: '{remote_id}'");
            }
            missing.commit()?;
            Ok(None)
        }
        RecordRowResponse::NullAlias(alias) => {
            if !ignore_null {
                error!("Undefined alias: '{alias}'");
            }
            Ok(None)
        }
        RecordRowResponse::InvalidRemoteId(err) => {
            reraise(&err);
            Ok(None)
        }
    }
}

/// Validate a BibTeX key, logging errors and suggesting fixes.
fn validate_bibtex_key(key: String, row: &State<EntryRow>) -> Option<EntryKey<String>> {
    match EntryKey::try_new(key) {
        Ok(bibtex_key) => Some(bibtex_key),
        Err(parse_result) => {
            match row.get_valid_referencing_keys() {
                Ok(alternative_keys) => {
                    if !alternative_keys.is_empty() {
                        reraise(&parse_result);
                        suggest!(
                            "Use one of the following equivalent keys: {}",
                            alternative_keys.join(", ")
                        );
                    } else {
                        reraise(&parse_result);
                        suggest!(
                            "Create an alias which does not contain whitespace or disallowed characters: {{}}(),=\\#%\""
                        );
                    }
                }
                Err(error2) => {
                    reraise(&parse_result);
                    error!("Another error occurred while retrieving equivalent keys!");
                    reraise(&error2);
                }
            }
            None
        }
    }
}
