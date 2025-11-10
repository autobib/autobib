mod key;
mod mapped;

use anyhow::bail;
use nonempty::NonEmpty;

pub use self::key::{Alias, AliasOrRemoteId, MappedAliasOrRemoteId, MappedKey, RecordId, RemoteId};
use crate::{
    Config,
    config::AliasTransform,
    db::{
        RecordDatabase,
        state::{
            Missing, NullRecordRow, RecordIdState, RecordRow, RemoteIdState, RowData, State,
            Unknown,
        },
    },
    entry::{MutableEntryData, RawEntryData},
    error::{Error, ProviderError, RecordError},
    http::Client,
    logger::info,
    normalize::{Normalization, Normalize},
    provider::{RemoteResponse, get_remote_response},
};

/// The fundamental record type.
#[derive(Debug)]
pub struct Record {
    /// The original key.
    pub key: String,
    /// The raw data.
    pub data: RawEntryData,
    /// The canonical identifier.
    pub canonical: RemoteId,
}

/// The response type of [`get_record_row_remote`].
///
/// If the record exists, the resulting [`State<RecordRow>`] is guaranteed to be valid for the row corresponding
/// to the [`Record`].
///
/// If the record does not exist, then the resulting [`State<NullRecordRow>`] is guaranteed to not exist in the
/// `Records` table, and be cached in the `NullRecords` table.
#[derive(Debug)]
pub enum RemoteRecordRowResponse<'conn> {
    /// The record exists.
    Exists(Record, State<'conn, RecordRow>),
    /// The record is null.
    Null(RemoteId, State<'conn, NullRecordRow>),
}

/// The response type of [`get_record_row`].
///
/// If the record exists, the resulting [`State<RecordRow>`] is guaranteed to be valid for the row corresponding
/// to the [`Record`].
///
/// If the record does not exist, then the resulting [`State<NullRecordRow>`] is guaranteed to not exist in the
/// `Records` table, and be cached in the `NullRecords` table.
///
/// The database state is passed back to the caller inside the enum. Note that this
/// transaction *must* be committed in order for database changes to be in effect, regardless if
/// the record exists or is null, since the null records are also cached inside the database.
#[derive(Debug)]
pub enum RecordRowResponse<'conn> {
    /// The record exists.
    Exists(Record, State<'conn, RecordRow>),
    /// The record is null.
    NullRemoteId(RemoteId, State<'conn, NullRecordRow>),
    /// The identifier has an invalid form.
    InvalidRemoteId(RecordError),
    /// The alias does not exist.
    NullAlias(Alias),
}

impl<'conn> From<RemoteRecordRowResponse<'conn>> for RecordRowResponse<'conn> {
    fn from(resp: RemoteRecordRowResponse<'conn>) -> Self {
        match resp {
            RemoteRecordRowResponse::Exists(record, state) => {
                RecordRowResponse::Exists(record, state)
            }
            RemoteRecordRowResponse::Null(remote_id, state) => {
                RecordRowResponse::NullRemoteId(remote_id, state)
            }
        }
    }
}

impl<'conn> RecordRowResponse<'conn> {
    /// Either return the record and corresponding state transaction wrapper, or raise an error. In
    /// order to commit the new changes, the resulting [`RecordRow`] must be committed.
    ///
    /// If the record is null, the corresponding transaction is automatically committed before
    /// returning the relevant error.
    pub fn exists_or_commit_null(
        self,
        err_prefix: &str,
    ) -> Result<(Record, State<'conn, RecordRow>), anyhow::Error> {
        match self {
            RecordRowResponse::Exists(record, row) => Ok((record, row)),
            RecordRowResponse::NullRemoteId(remote_id, null_row) => {
                null_row.commit()?;
                bail!("{err_prefix} null record '{remote_id}'");
            }
            RecordRowResponse::InvalidRemoteId(record_error) => {
                bail!(record_error);
            }
            RecordRowResponse::NullAlias(alias) => {
                bail!("{err_prefix} undefined alias '{alias}'");
            }
        }
    }
}

fn row_to_response<'conn, K: Into<String>, T: From<RemoteRecordRowResponse<'conn>>>(
    key: K,
    row: State<'conn, RecordRow>,
) -> Result<T, Error> {
    let RowData {
        data, canonical, ..
    } = row.get_data()?;
    Ok(RemoteRecordRowResponse::Exists(
        Record {
            key: key.into(),
            data,
            canonical,
        },
        row,
    )
    .into())
}

/// Get the [`Record`] associated with a [`RecordId`].
///
/// The database state is passed back to the caller and must be commited for the record to be
/// recorded in the database.
pub fn get_record_row<'conn, F, C>(
    db: &'conn mut RecordDatabase,
    record_id: RecordId,
    client: &C,
    config: &Config<F>,
) -> Result<RecordRowResponse<'conn>, Error>
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
{
    match db.state_from_record_id(record_id, &config.alias_transform)? {
        RecordIdState::Existent(key, row) => {
            info!("Found existing data for key {key}");
            row_to_response(key, row)
        }
        RecordIdState::NullRemoteId(remote_id, null_row) => {
            Ok(RecordRowResponse::NullRemoteId(remote_id.mapped, null_row))
        }
        RecordIdState::UndefinedAlias(alias) => Ok(RecordRowResponse::NullAlias(alias)),
        RecordIdState::InvalidRemoteId(err) => Ok(RecordRowResponse::InvalidRemoteId(err)),
        RecordIdState::Unknown(Unknown::MappedAlias(alias, mapped, missing)) => {
            get_record_row_recursive(missing, mapped, client, &config.on_insert, |row| {
                // create the new alias
                if config.alias_transform.create() {
                    row.add_alias(&alias)?;
                }
                Ok(Some(alias.into()))
            })
            .map(Into::into)
        }
        RecordIdState::Unknown(Unknown::RemoteId(maybe_normalized, missing)) => {
            get_record_row_recursive(
                missing,
                maybe_normalized.mapped,
                client,
                &config.on_insert,
                |_| Ok(maybe_normalized.original),
            )
            .map(Into::into)
        }
    }
}

/// Get the [`Record`] associated with a [`RemoteId`].
pub fn get_record_row_remote<'conn, F, C>(
    db: &'conn mut RecordDatabase,
    remote_id: RemoteId,
    client: &C,
    config: &Config<F>,
) -> Result<RemoteRecordRowResponse<'conn>, Error>
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    C: Client,
{
    match db.state_from_remote_id(&remote_id)? {
        RemoteIdState::Existent(row) => {
            info!("Found existing data for key {remote_id}");
            row_to_response(remote_id, row)
        }
        RemoteIdState::Null(null_row) => Ok(RemoteRecordRowResponse::Null(remote_id, null_row)),
        RemoteIdState::Unknown(missing) => {
            get_record_row_recursive(missing, remote_id, client, &config.on_insert, |_| Ok(None))
        }
    }
}

/// Destructure a [`NonEmpty`] and return the last element.
#[inline]
fn into_last<T>(ne: NonEmpty<T>) -> T {
    let NonEmpty { head, mut tail } = ne;
    tail.pop().unwrap_or(head)
}

/// Resolve remote records inside a loop within a transaction.
///
/// The `exists_callback` is called if the remote record exists, and is passed a reference to the
/// row which will eventually be returned. The closure can optionally return a string which
/// will be used as the bibtex key in the resulting returned [`Record`]. If the closure does not
/// returns nothing, the original [`RemoteId`] is used as the bibtex key.
///
/// At each intermediate stage, attempt to read any data possible from the database
/// inside the transaction implicit in the [`State<Missing>`], and write any new data to the
/// database.
fn get_record_row_recursive<'conn, C: Client>(
    mut missing: State<'conn, Missing>,
    remote_id: RemoteId,
    client: &C,
    normalization: &Normalization,
    exists_callback: impl FnOnce(&State<'conn, RecordRow>) -> Result<Option<String>, rusqlite::Error>,
) -> Result<RemoteRecordRowResponse<'conn>, Error> {
    info!("Resolving remote record for {remote_id}");
    let mut history = NonEmpty::singleton(remote_id);
    loop {
        missing = match get_remote_response(client, history.last())? {
            RemoteResponse::Data(mut data) => {
                data.normalize(normalization);
                let raw_record_data = RawEntryData::from_entry_data(&data);

                // SAFETY: the provided canonical identifier is present in the provided references
                let row = unsafe {
                    missing.insert_with_refs(&raw_record_data, history.last(), history.iter())?
                };
                let original = exists_callback(&row)?;

                let NonEmpty { head, mut tail } = history;
                let (key, canonical) = match (original, tail.pop()) {
                    (Some(key), Some(canonical)) => (key, canonical),
                    (Some(key), None) => (key, head),
                    (None, Some(canonical)) => (head.into(), canonical),
                    (None, None) => (head.to_string(), head),
                };

                break Ok(RemoteRecordRowResponse::Exists(
                    Record {
                        key,
                        data: RawEntryData::from_entry_data(&data),
                        canonical,
                    },
                    row,
                ));
            }
            RemoteResponse::Reference(new_remote_id) => match missing.reset(&new_remote_id)? {
                RemoteIdState::Existent(row) => {
                    // not necessary to insert `new_remote_id` since we just saw that it
                    // is present in the database
                    row.add_refs(history.iter())?;
                    let RowData {
                        data, canonical, ..
                    } = row.get_data()?;
                    let original = exists_callback(&row)?;
                    break Ok(RemoteRecordRowResponse::Exists(
                        Record {
                            key: original.unwrap_or(history.head.into()),
                            data,
                            canonical,
                        },
                        row,
                    ));
                }
                RemoteIdState::Null(null_records_row) => {
                    null_records_row.commit()?;
                    break Err(ProviderError::UnexpectedNullRemoteFromProvider(
                        new_remote_id.into(),
                    )
                    .into());
                }
                RemoteIdState::Unknown(missing) => {
                    history.push(new_remote_id);
                    missing
                }
            },
            RemoteResponse::Null => {
                if history.tail.is_empty() {
                    let remote_id = into_last(history);
                    let null_row = missing.set_null(&remote_id)?;
                    break Ok(RemoteRecordRowResponse::Null(remote_id, null_row));
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
    Exists(MutableEntryData, RemoteId),
    /// The remote record does not exist.
    Null(RemoteId),
}

/// Get the [`Record`] associated with a [`RemoteId`], or [`None`] if the [`Record`] does not exist.
///
/// This method does not involve any database reads or writes, and simply loops to obtain the
/// remote record associated with a [`RemoteId`].
pub fn get_remote_response_recursive<C: Client>(
    remote_id: RemoteId,
    client: &C,
) -> Result<RecursiveRemoteResponse, Error> {
    info!("Resolving remote record for '{remote_id}'");
    let mut history = NonEmpty::singleton(remote_id);
    loop {
        let last = history.last();

        match get_remote_response(client, last)? {
            RemoteResponse::Data(data) => {
                break Ok(RecursiveRemoteResponse::Exists(data, into_last(history)));
            }
            RemoteResponse::Reference(new_remote_id) => {
                history.push(new_remote_id);
            }
            RemoteResponse::Null => {
                break Ok(RecursiveRemoteResponse::Null(history.head));
            }
        }
    }
}
