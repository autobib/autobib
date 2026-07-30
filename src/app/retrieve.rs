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
        state::{DatabaseResponse, IsEntry, Record, State},
    },
    entry::{BibtexEntry, EntryKey},
    error::{DatabaseError, Error},
    http::Client,
    logger::{error, reraise, suggest},
    record::{Identifier, Key, KeyedRecord, RecordResponse, get_record},
};

/// Retrieve identifiers as BibTeX entries.
pub fn retrieve_entries<T: IntoIterator<Item = Key>, C: Client>(
    ids: T,
    record_db: &mut RecordDatabase,
    client: &C,
    ignore_null: bool,
    config: &Config,
) -> BTreeMap<Identifier, NonEmpty<BibtexEntry>> {
    let valid_entries = ids.into_iter().filter_map(|id| {
        retrieve_single_entry(record_db, id, client, ignore_null, config, |r, s| {
            Ok(try_data_to_entry(r, s))
        })
        .unwrap_or_else(|error| {
            reraise(&error);
            None
        })
    });
    group_valid_entries_by_canonical(valid_entries)
}

/// Synchronize entries with remote.
pub fn sync_entries<T: IntoIterator<Item = Key>, C: Client>(
    ids: T,
    record_db: &mut RecordDatabase,
    client: &C,
    ignore_null: bool,
    config: &Config,
) {
    for id in ids {
        retrieve_single_entry(record_db, id, client, ignore_null, config, |_, _| {
            Ok(Option::<Infallible>::None)
        })
        .unwrap_or_else(|error| {
            reraise(&error);
            None
        });
    }
}

/// Retrieve identifiers as BibTeX entries without writing to the database or making remote
/// requests.
pub fn retrieve_entries_read_only<T: IntoIterator<Item = Key>>(
    ids: T,
    record_db: &mut RecordDatabase,
    ignore_null: bool,
    config: &Config,
) -> BTreeMap<Identifier, NonEmpty<BibtexEntry>> {
    let valid_entries = ids.into_iter().filter_map(|key| {
        retrieve_single_entry_read_only(record_db, key, ignore_null, config, |r, s| {
            Ok(try_data_to_entry(r, s))
        })
        .unwrap_or_else(|error| {
            error!("{error}");
            None
        })
    });
    group_valid_entries_by_canonical(valid_entries)
}

/// Synchronize entries with remote.
pub fn sync_entries_read_only<T: IntoIterator<Item = Key>>(
    ids: T,
    record_db: &mut RecordDatabase,
    ignore_null: bool,
    config: &Config,
) {
    for id in ids {
        retrieve_single_entry_read_only(record_db, id, ignore_null, config, |_, _| {
            Ok(Option::<Infallible>::None)
        })
        .unwrap_or_else(|error| {
            reraise(&error);
            None
        });
    }
}

/// Retrieve a single entry and apply a closure to the resulting data.
pub fn retrieve_single_entry_read_only<V, T>(
    record_db: &mut RecordDatabase,
    id: Key,
    ignore_null: bool,
    config: &Config,
    validate: V,
) -> Result<Option<T>, Error>
where
    V: FnOnce(KeyedRecord, &State<'_, IsEntry>) -> Result<Option<T>, DatabaseError>,
{
    match record_db.state_from_key(id, &config.alias_transform)? {
        DatabaseResponse::Entry(record, state) => {
            let entry = validate(record, &state)?;
            state.commit()?;
            Ok(entry)
        }
        DatabaseResponse::Deleted(
            KeyedRecord {
                key,
                record: deleted_row_data,
            },
            state,
        ) => {
            if !ignore_null {
                error!("Deleted record: '{key}'");
                if let Some(repl) = deleted_row_data.data {
                    suggest!("Use the replacement key '{repl}'");
                }
            }
            state.commit()?;
            Ok(None)
        }
        DatabaseResponse::Void(KeyedRecord { key, .. }, void) => {
            void.commit()?;
            error!("Record exists but has been voided: {key}");
            Ok(None)
        }
        DatabaseResponse::NullId(id, missing) => {
            if !ignore_null {
                error!("Null record: '{id}'");
            }
            missing.commit()?;
            Ok(None)
        }
        DatabaseResponse::UndefinedAlias(alias) => {
            if !ignore_null {
                error!("Undefined alias: '{alias}'");
            }
            Ok(None)
        }
        DatabaseResponse::InvalidId(err) => {
            reraise(&err);
            Ok(None)
        }
        DatabaseResponse::Unknown(unknown) => {
            let mapped = unknown.combine_and_commit()?;
            error!("Database does not contain key: {mapped}");
            Ok(None)
        }
    }
}

/// Retrieve and apply a validation function to a single record
pub fn retrieve_single_entry<C, V, T>(
    record_db: &mut RecordDatabase,
    id: Key,
    client: &C,
    ignore_null: bool,
    config: &Config,
    validate: V,
) -> Result<Option<T>, Error>
where
    C: Client,
    V: FnOnce(KeyedRecord, &State<'_, IsEntry>) -> Result<Option<T>, DatabaseError>,
{
    match get_record(record_db, id, client, config)? {
        RecordResponse::Exists(record_data, row) => {
            let entry = validate(record_data, &row)?;
            row.commit()?;
            Ok(entry)
        }
        RecordResponse::Deleted(deleted_row_data, deleted) => {
            if !ignore_null {
                error!("Deleted record: '{}'", deleted_row_data.key);
                if let Some(repl) = deleted_row_data.record.data {
                    suggest!("Perhaps use the replacement key: '{repl}'");
                }
            }
            deleted.commit()?;
            Ok(None)
        }
        RecordResponse::NullId(id, missing) => {
            if !ignore_null {
                error!("Null record: '{id}'");
            }
            missing.commit()?;
            Ok(None)
        }
        RecordResponse::NullAlias(alias) => {
            if !ignore_null {
                error!("Undefined alias: '{alias}'");
            }
            Ok(None)
        }
        RecordResponse::InvalidId(err) => {
            reraise(&err);
            Ok(None)
        }
    }
}

/// Helper function for converting data to an entry with validation.
pub fn try_data_to_entry<D, S: AsRef<str>>(
    KeyedRecord {
        key,
        record: Record {
            data, canonical, ..
        },
    }: KeyedRecord<Record<D, S>, S>,
    row: &State<'_, IsEntry>,
) -> Option<(BibtexEntry<D, S>, Identifier<S>)> {
    validate_bibtex_key(key, row).map(|key| (BibtexEntry::new(key, data), canonical))
}

/// Validate a BibTeX key, logging errors and suggesting fixes.
fn validate_bibtex_key<S: AsRef<str>>(key: S, row: &State<IsEntry>) -> Option<EntryKey<S>> {
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
fn group_valid_entries_by_canonical<T, D, S, I: Ord>(
    valid_entries: T,
) -> BTreeMap<Identifier<I>, NonEmpty<BibtexEntry<D, S>>>
where
    T: IntoIterator<Item = (BibtexEntry<D, S>, Identifier<I>)>,
{
    let mut grouped_entries: BTreeMap<Identifier<I>, NonEmpty<BibtexEntry<D, S>>> = BTreeMap::new();
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
