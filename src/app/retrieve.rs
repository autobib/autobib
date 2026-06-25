use std::{
    collections::{
        BTreeMap,
        btree_map::Entry::{Occupied, Vacant},
    },
    convert::Infallible,
};

use nonempty::NonEmpty;
use serde_bibtex::token::is_entry_key;

use crate::{
    config::Config,
    db::{
        RecordDatabase,
        state::{IsEntry, RecordIdState, RecordRow, State},
    },
    entry::{Entry, EntryKey, RawEntryData},
    error::Error,
    http::Client,
    logger::{error, reraise, suggest},
    record::{Record, RecordId, RecordRowResponse, RemoteId, get_record_row},
};

/// Retrieve identifiers as BibTeX entries.
pub fn retrieve_entries<
    T: IntoIterator<Item = RecordId>,
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
>(
    ids: T,
    record_db: &mut RecordDatabase,
    client: &C,
    ignore_null: bool,
    config: &Config<F>,
) -> BTreeMap<RemoteId, NonEmpty<Entry<RawEntryData>>> {
    let valid_entries = ids.into_iter().filter_map(|id| {
        retrieve_single_entry(
            record_db,
            id,
            client,
            ignore_null,
            config,
            try_data_to_entry,
        )
        .unwrap_or_else(|error| {
            reraise(&error);
            None
        })
    });
    group_valid_entries_by_canonical(valid_entries)
}

/// Synchronize entries with remote.
pub fn sync_entries<
    T: IntoIterator<Item = RecordId>,
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
>(
    ids: T,
    record_db: &mut RecordDatabase,
    client: &C,
    ignore_null: bool,
    config: &Config<F>,
) {
    for id in ids {
        retrieve_single_entry(record_db, id, client, ignore_null, config, |_, _| {
            Option::<Infallible>::None
        })
        .unwrap_or_else(|error| {
            reraise(&error);
            None
        });
    }
}

/// Retrieve identifiers as BibTeX entries without writing to the database or making remote
/// requests.
pub fn retrieve_entries_read_only<
    T: IntoIterator<Item = RecordId>,
    F: FnOnce() -> Vec<(regex::Regex, String)>,
>(
    ids: T,
    record_db: &mut RecordDatabase,
    ignore_null: bool,
    config: &Config<F>,
) -> BTreeMap<RemoteId, NonEmpty<Entry<RawEntryData>>> {
    let valid_entries = ids.into_iter().filter_map(|record_id| {
        retrieve_single_entry_read_only(
            record_db,
            record_id,
            ignore_null,
            config,
            try_data_to_entry,
        )
        .unwrap_or_else(|error| {
            error!("{error}");
            None
        })
    });
    group_valid_entries_by_canonical(valid_entries)
}

/// Synchronize entries with remote.
pub fn sync_entries_read_only<
    T: IntoIterator<Item = RecordId>,
    F: FnOnce() -> Vec<(regex::Regex, String)>,
>(
    ids: T,
    record_db: &mut RecordDatabase,
    ignore_null: bool,
    config: &Config<F>,
) {
    for id in ids {
        retrieve_single_entry_read_only(record_db, id, ignore_null, config, |_, _| {
            Option::<Infallible>::None
        })
        .unwrap_or_else(|error| {
            reraise(&error);
            None
        });
    }
}

/// Retrieve a single entry and apply a closure to the resulting data.
pub fn retrieve_single_entry_read_only<F, V, T>(
    record_db: &mut RecordDatabase,
    id: RecordId,
    ignore_null: bool,
    config: &Config<F>,
    validate: V,
) -> Result<Option<T>, Error>
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    V: FnOnce(Record<RawEntryData>, &State<'_, IsEntry>) -> Option<T>,
{
    match record_db.state_from_record_id(id, &config.alias_transform)? {
        RecordIdState::Entry(
            key,
            RecordRow::<RawEntryData> {
                data, canonical, ..
            },
            state,
        ) => {
            let entry = validate(
                Record {
                    key,
                    data,
                    canonical,
                },
                &state,
            );
            state.commit()?;
            Ok(entry)
        }
        RecordIdState::Deleted(key, deleted_row_data, state) => {
            if !ignore_null {
                error!("Deleted record: '{key}'");
                if let Some(repl) = deleted_row_data.data {
                    suggest!("Use the replacement key '{repl}'");
                }
            }
            state.commit()?;
            Ok(None)
        }
        RecordIdState::Void(key, _, void) => {
            void.commit()?;
            error!("Record exists but has been voided: {key}");
            Ok(None)
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

/// Retrieve and apply a validation function to a single record
pub fn retrieve_single_entry<F, C, V, T>(
    record_db: &mut RecordDatabase,
    id: RecordId,
    client: &C,
    ignore_null: bool,
    config: &Config<F>,
    validate: V,
) -> Result<Option<T>, Error>
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
    V: FnOnce(Record<RawEntryData>, &State<'_, IsEntry>) -> Option<T>,
{
    match get_record_row(record_db, id, client, config)? {
        RecordRowResponse::Exists(record_data, row) => {
            let entry = validate(record_data, &row);
            row.commit()?;
            Ok(entry)
        }
        RecordRowResponse::Deleted(deleted_row_data, deleted) => {
            if !ignore_null {
                error!("Deleted record: '{}'", deleted_row_data.key);
                if let Some(repl) = deleted_row_data.data {
                    suggest!("Perhaps use the replacement key: '{repl}'");
                }
            }
            deleted.commit()?;
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

/// Helper function for converting data to an entry with validation.
pub fn try_data_to_entry(
    Record {
        key,
        data,
        canonical,
    }: Record<RawEntryData>,
    row: &State<'_, IsEntry>,
) -> Option<(Entry<RawEntryData>, RemoteId)> {
    validate_bibtex_key(key, row).map(|key| (Entry::new(key, data), canonical))
}

/// Validate a BibTeX key, logging errors and suggesting fixes.
fn validate_bibtex_key(key: String, row: &State<IsEntry>) -> Option<EntryKey<String>> {
    match EntryKey::try_new(key) {
        Ok(bibtex_key) => Some(bibtex_key),
        Err(parse_result) => {
            match row.referencing_keys() {
                Ok(mut alternative_keys) => {
                    alternative_keys.retain(|k| is_entry_key(k));

                    reraise(&parse_result);
                    if !alternative_keys.is_empty() {
                        suggest!(
                            "Use one of the following equivalent keys: {}",
                            alternative_keys.join(", ")
                        );
                    } else {
                        suggest!(
                            "Create an alias which does not contain whitespace or disallowed characters: {{}}(),=\\#%\""
                        );
                    };
                }
                Err(e) => {
                    reraise(&e);
                }
            };
            None
        }
    }
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
