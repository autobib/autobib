mod key;

use anyhow::bail;
use nonempty::NonEmpty;

pub use self::key::{Alias, AliasOrId, Identifier, Key, LegacyAlias, MappedAliasOrId, MappedKey};
use crate::{
    Config,
    config::AliasTransform,
    db::{
        RecordDatabase, Tx,
        state::{
            DatabaseIdResponse, DatabaseResponse, IsDeleted, IsEntry, IsMissing, IsNull, IsVoid,
            Record, State, Unknown, Updated,
        },
    },
    entry::{MutableEntryData, RawEntryData},
    error::{Error, ProviderError, RecordError},
    http::Client,
    logger::info,
    normalize::{Normalization, Normalize},
    provider::{RemoteResponse, get_remote_response},
};

/// The fundamental record type for a record in the 'Records' table, with data depending on the
/// data type of the row.
#[derive(Debug)]
pub struct KeyedRecord<D> {
    /// The original key.
    pub key: String,
    /// The record.
    pub record: Record<D>,
}

impl<D> From<KeyedRecord<D>> for Record<D> {
    fn from(record: KeyedRecord<D>) -> Self {
        record.record
    }
}

/// The response type of [`get_record`].
///
/// If the record exists, the resulting [`State<RecordRow>`] is guaranteed to be valid for the row corresponding
/// to the [`KeyedRecord`].
///
/// If the record does not exist, then the resulting [`State<IsNull>`] is guaranteed to not exist in the
/// `Records` table, and not be cached in the `NullRecords` table.
///
/// The database state is passed back to the caller inside the enum. Note that this
/// transaction *must* be committed in order for database changes to be in effect, regardless if
/// the record exists or is null, since the null records are also cached inside the database.
#[derive(Debug)]
pub enum RecordResponse<'conn> {
    /// The record exists.
    Exists(KeyedRecord<RawEntryData>, State<'conn, IsEntry>),
    /// The record was deleted.
    Deleted(KeyedRecord<Option<Identifier>>, State<'conn, IsDeleted>),
    /// The record is null.
    NullId(Identifier, State<'conn, IsNull>),
    /// The identifier has an invalid form.
    InvalidId(RecordError),
    /// The alias does not exist.
    NullAlias(Alias),
}

impl<'conn> RecordResponse<'conn> {
    /// Either return the record and corresponding state transaction wrapper, or raise an error. In
    /// order to commit the new changes, the resulting [`State`] must be committed.
    ///
    /// If the record is null, the corresponding transaction is automatically committed before
    /// returning the relevant error.
    pub fn exists_or(
        self,
        f: impl FnOnce(Tx<'conn>) -> rusqlite::Result<()>,
        err_prefix: impl std::fmt::Display,
    ) -> Result<(KeyedRecord<RawEntryData>, State<'conn, IsEntry>), anyhow::Error> {
        match self {
            RecordResponse::Exists(record, row) => Ok((record, row)),
            RecordResponse::Deleted(data, deleted_row) => {
                f(deleted_row.into_tx())?;
                if let Some(repl) = data.record.data {
                    bail!(
                        "{err_prefix} deleted record '{}' (replaced by key '{repl}')",
                        data.key
                    );
                } else {
                    bail!("{err_prefix} deleted record '{}'", data.key);
                }
            }
            RecordResponse::NullId(id, null_row) => {
                f(null_row.into_tx())?;
                bail!("{err_prefix} null record '{id}'");
            }
            RecordResponse::InvalidId(record_error) => {
                bail!(record_error);
            }
            RecordResponse::NullAlias(alias) => {
                bail!("{err_prefix} undefined alias '{alias}'");
            }
        }
    }

    /// Either return the record and corresponding state transaction wrapper, or raise an error. In
    /// order to commit the new changes, the resulting [`State`] must be committed.
    ///
    /// If the record is null, the corresponding transaction is automatically committed before
    /// returning the relevant error.
    pub fn exists_or_commit_null(
        self,
        err_prefix: &str,
    ) -> Result<(KeyedRecord<RawEntryData>, State<'conn, IsEntry>), anyhow::Error> {
        match self {
            RecordResponse::Exists(record, row) => Ok((record, row)),
            RecordResponse::Deleted(data, deleted_row) => {
                deleted_row.commit()?;
                if let Some(repl) = data.record.data {
                    bail!(
                        "{err_prefix} deleted record '{}' (replaced by key '{repl}')",
                        data.key
                    );
                } else {
                    bail!("{err_prefix} deleted record '{}'", data.key);
                }
            }
            RecordResponse::NullId(id, null_row) => {
                null_row.commit()?;
                bail!("{err_prefix} null record '{id}'");
            }
            RecordResponse::InvalidId(record_error) => {
                bail!(record_error);
            }
            RecordResponse::NullAlias(alias) => {
                bail!("{err_prefix} undefined alias '{alias}'");
            }
        }
    }
}

pub fn get_record_tx<'conn, C>(
    tx: Tx<'conn>,
    key: Key,
    client: &C,
    config: &Config,
) -> Result<RecordResponse<'conn>, Error>
where
    C: Client,
{
    match DatabaseResponse::determine(tx, key, &config.alias_transform)? {
        DatabaseResponse::Entry(record, state) => {
            info!("Found existing data for key {}", record.key);
            Ok(RecordResponse::Exists(record, state))
        }
        DatabaseResponse::Deleted(record, state) => Ok(RecordResponse::Deleted(record, state)),
        DatabaseResponse::NullId(id, null_row) => Ok(RecordResponse::NullId(id.mapped, null_row)),
        DatabaseResponse::UndefinedAlias(alias) => Ok(RecordResponse::NullAlias(alias)),
        DatabaseResponse::InvalidId(err) => Ok(RecordResponse::InvalidId(err)),
        DatabaseResponse::Void(KeyedRecord { key, record }, void) => {
            let (raw_entry_data, updated) =
                revive_void(void, &record.canonical, client, &config.on_insert)?;
            Ok(RecordResponse::Exists(
                KeyedRecord {
                    key,
                    record: Record {
                        canonical: record.canonical,
                        data: raw_entry_data,
                        modified: updated.modified,
                    },
                },
                updated.state,
            ))
        }
        DatabaseResponse::Unknown(Unknown::MappedAlias(alias, mapped, missing)) => {
            get_record_recursive(
                missing,
                mapped,
                client,
                &config.on_insert,
                |row, alias| {
                    // create the new alias
                    if config.alias_transform.create() {
                        row.add_alias(&alias)?;
                    }
                    Ok(Some(alias.into()))
                },
                |_, alias| Ok(Some(alias.into())),
                alias,
            )
        }
        DatabaseResponse::Unknown(Unknown::Id(maybe_normalized, missing)) => get_record_recursive(
            missing,
            maybe_normalized.mapped,
            client,
            &config.on_insert,
            |_, t| Ok(t),
            |_, t| Ok(t),
            maybe_normalized.original,
        ),
    }
}

/// Get the [`Record`] associated with a [`Key`].
///
/// The database state is passed back to the caller and must be commited for the record to be
/// recorded in the database.
pub fn get_record<'conn, C>(
    db: &'conn mut RecordDatabase,
    key: Key,
    client: &C,
    config: &Config,
) -> Result<RecordResponse<'conn>, Error>
where
    C: Client,
{
    get_record_tx(db.transaction()?, key, client, config)
}

/// Destructure a [`NonEmpty`] and return the last element.
#[inline]
fn into_last<T>(NonEmpty { head, mut tail }: NonEmpty<T>) -> T {
    tail.pop().unwrap_or(head)
}

/// Resolve remote records inside a loop within a transaction.
///
/// The `exists_callback` is called if the remote record exists, and is passed a reference to the
/// row which will eventually be returned. The closure can optionally return a string which
/// will be used as the bibtex key in the resulting returned [`Record`]. If the closure
/// returns `None`, the original [`Identifier`] is used as the bibtex key.
///
/// The `deleted_callback` is called if the record exists in the database, but it was deleted.
///
/// At each intermediate stage, attempt to read any data possible from the database
/// inside the transaction implicit in the [`State<Missing>`], and write any new data to the
/// database.
fn get_record_recursive<'conn, O, C: Client>(
    mut missing: State<'conn, IsMissing>,
    id: Identifier,
    client: &C,
    normalization: &Normalization,
    exists_callback: impl FnOnce(&State<'conn, IsEntry>, O) -> Result<Option<String>, rusqlite::Error>,
    deleted_callback: impl FnOnce(
        &State<'conn, IsDeleted>,
        O,
    ) -> Result<Option<String>, rusqlite::Error>,
    original: O,
) -> Result<RecordResponse<'conn>, Error> {
    info!("Resolving remote record for {id}");
    let mut history = NonEmpty::singleton(id);
    loop {
        missing = match get_remote_response(client, history.last())? {
            RemoteResponse::Data(mut data) => {
                data.normalize(normalization);
                let raw_record_data = RawEntryData::from_entry_data(&data);

                // SAFETY: the provided canonical identifier is present in the provided references
                let row =
                    missing.insert_with_refs(&raw_record_data, history.last(), history.iter())?;
                let maybe_key = exists_callback(&row.state, original)?;

                let NonEmpty { head, mut tail } = history;
                let (key, canonical) = match (maybe_key, tail.pop()) {
                    (Some(key), Some(canonical)) => (key, canonical),
                    (Some(key), None) => (key, head),
                    (None, Some(canonical)) => (head.into(), canonical),
                    (None, None) => (head.to_string(), head),
                };

                break Ok(RecordResponse::Exists(
                    KeyedRecord {
                        key,
                        record: Record {
                            data: RawEntryData::from_entry_data(&data),
                            canonical,
                            modified: row.modified,
                        },
                    },
                    row.state,
                ));
            }
            RemoteResponse::Reference(new_id) => {
                match DatabaseIdResponse::determine(missing.into_tx(), &new_id)? {
                    DatabaseIdResponse::Entry(record, state) => {
                        // not necessary to insert `new_id` since we just saw that it
                        // is present in the database
                        state.add_refs(history.iter())?;
                        let maybe_key = exists_callback(&state, original)?;
                        break Ok(RecordResponse::Exists(
                            KeyedRecord {
                                key: maybe_key.unwrap_or(history.head.into()),
                                record,
                            },
                            state,
                        ));
                    }
                    DatabaseIdResponse::Deleted(record, state) => {
                        // we still add the refs to the deleted row
                        state.add_refs(history.iter())?;
                        let maybe_key = deleted_callback(&state, original)?;
                        break Ok(RecordResponse::Deleted(
                            KeyedRecord {
                                key: maybe_key.unwrap_or(history.head.into()),
                                record,
                            },
                            state,
                        ));
                    }
                    DatabaseIdResponse::Null(null_records_row) => {
                        null_records_row.commit()?;
                        break Err(
                            ProviderError::UnexpectedNullRemoteFromProvider(new_id.into()).into(),
                        );
                    }
                    DatabaseIdResponse::Unknown(missing) => {
                        history.push(new_id);
                        missing
                    }
                    DatabaseIdResponse::Void(record_row, void) => {
                        // use the special 'lookup and reinsert' method
                        let (data, entry) =
                            revive_void(void, &record_row.canonical, client, normalization)?;
                        // add the new references
                        entry.state.add_refs(history.iter())?;
                        let key =
                            exists_callback(&entry.state, original)?.unwrap_or(history.head.into());

                        break Ok(RecordResponse::Exists(
                            KeyedRecord {
                                key,
                                record: Record {
                                    data,
                                    canonical: record_row.canonical,
                                    modified: entry.modified,
                                },
                            },
                            entry.state,
                        ));
                    }
                }
            }
            RemoteResponse::Null => {
                if history.tail.is_empty() {
                    let id = into_last(history);
                    let null_row = missing.set_null(&id)?;
                    break Ok(RecordResponse::NullId(id, null_row));
                } else {
                    break Err(ProviderError::UnexpectedNullRemoteFromProvider(
                        into_last(history).into(),
                    )
                    .into());
                }
            }
        };
    }
}

/// The result of obtaining a remote record, with no reference to a database.
pub enum RecursiveRemoteResponse {
    /// The remote record exists, and has the provided data and canonical identifier.
    Exists(MutableEntryData, Identifier),
    /// The remote record does not exist.
    Null(Identifier),
}

/// Revive a void record by retrieving the canonical data and re-inserting the record.
pub fn revive_void<'conn, C: Client>(
    void: State<'conn, IsVoid>,
    canonical: &Identifier,
    client: &C,
    normalization: &Normalization,
) -> Result<(RawEntryData, Updated<'conn, IsEntry>), Error> {
    match get_remote_response(client, canonical)? {
        RemoteResponse::Data(mut mutable_entry_data) => {
            mutable_entry_data.normalize(normalization);
            let data = RawEntryData::from_entry_data(&mutable_entry_data);
            let entry = void.reinsert(&data)?;
            Ok((data, entry))
        }
        RemoteResponse::Reference(id) => {
            panic!(
                "Database error: 'Records' table contains identifier {id} which is not canonical"
            );
        }
        RemoteResponse::Null => {
            Err(ProviderError::UnexpectedNullFromPreviousData(canonical.to_string()).into())
        }
    }
}

/// Get the [`Record`] associated with an [`Identifier`], or [`None`] if the [`Record`] does not exist.
///
/// This method does not involve any database reads or writes, and simply loops to obtain the
/// remote record associated with an [`Identifier`].
pub fn get_remote_response_recursive<C: Client>(
    id: Identifier,
    client: &C,
) -> Result<RecursiveRemoteResponse, Error> {
    info!("Resolving remote record for '{id}'");
    let mut history = NonEmpty::singleton(id);
    loop {
        let last = history.last();

        match get_remote_response(client, last)? {
            RemoteResponse::Data(data) => {
                break Ok(RecursiveRemoteResponse::Exists(data, into_last(history)));
            }
            RemoteResponse::Reference(new_id) => {
                history.push(new_id);
            }
            RemoteResponse::Null => {
                break Ok(RecursiveRemoteResponse::Null(history.head));
            }
        }
    }
}
