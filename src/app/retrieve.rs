use std::collections::{
    BTreeMap, HashMap, HashSet,
    btree_map::Entry::{Occupied, Vacant},
};

use nonempty::NonEmpty;

use crate::{
    config::Config,
    db::{
        RecordDatabase,
        state::{EntryRecordRow, NullRecordRow, RecordIdState, RowData, State},
    },
    entry::{Entry, EntryKey, RawEntryData},
    error::Error,
    http::Client,
    logger::{error, reraise, suggest},
    record::{Record, RecordRowResponse},
    record::{RecordId, RemoteId, get_record_row},
};

/// Lookup citation keys from the database, filtering out unknown and invalid remote ids and
/// undefined aliases.
///
/// The resulting hash map has keys which are the set of all unique canonical identifiers
/// corresponding to those citation keys which were present in the database, and values which are
/// the corresponding referencing citation keys which were initially present in the list.
///
/// The resulting hash set contains all of the null identifiers.
pub fn filter_and_deduplicate_by_canonical<T, N, F: FnOnce() -> Vec<(regex::Regex, String)>>(
    citation_keys: T,
    record_db: &mut RecordDatabase,
    ignore_errors: bool,
    mut null_callback: N,
    config: &Config<F>,
) -> Result<HashMap<RemoteId, HashSet<String>>, rusqlite::Error>
where
    T: Iterator<Item = RecordId>,
    N: FnMut(RemoteId, State<NullRecordRow>) -> Result<(), rusqlite::Error>,
{
    let mut deduplicated = HashMap::new();

    for record_id in citation_keys {
        match record_db.state_from_record_id(record_id, &config.alias_transform)? {
            RecordIdState::Entry(key, row) => {
                deduplicated
                    .entry(row.get_canonical()?)
                    .or_insert_with(HashSet::new)
                    .insert(key);
                row.commit()?;
            }
            RecordIdState::NullRemoteId(mapped_remote_id, null_row) => {
                null_callback(mapped_remote_id.mapped, null_row)?;
            }
            RecordIdState::Unknown(unknown) => {
                let maybe_normalized = unknown.combine_and_commit()?;
                if !ignore_errors {
                    error!("Identifier not in database: {maybe_normalized}");
                }
            }
            RecordIdState::UndefinedAlias(alias) => {
                if !ignore_errors {
                    error!("Undefined alias: '{alias}'");
                }
            }
            RecordIdState::InvalidRemoteId(err) => {
                if !ignore_errors {
                    reraise(&err);
                }
            }
        }
    }
    Ok(deduplicated)
}

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
                let RowData {
                    data, canonical, ..
                } = row.get_data()?;
                let entry =
                    validate_bibtex_key(key, &row).map(|key| (Entry::new(key, data), canonical));
                row.commit()?;
                Ok(entry)
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
fn validate_bibtex_key(key: String, row: &State<EntryRecordRow>) -> Option<EntryKey<String>> {
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
