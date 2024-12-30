mod key;

use anyhow::bail;
use nonempty::NonEmpty;

pub use self::key::{Alias, AliasOrRemoteId, MappedKey, RecordId, RemoteId};
use crate::{
    db::{
        state::{Missing, NullRecordRow, RecordIdState, RecordRow, RemoteIdState, RowData, State},
        RawRecordData, RecordData, RecordDatabase,
    },
    error::{Error, ProviderError, RecordError},
    logger::info,
    normalize::{Normalization, Normalize},
    provider::{get_remote_response, RemoteResponse},
    HttpClient,
};

/// The fundamental record type.
#[derive(Debug)]
pub struct Record {
    /// The original key.
    pub key: String,
    /// The raw data.
    pub data: RawRecordData,
    /// The canonical identifier.
    pub canonical: RemoteId,
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

/// Get the [`Record`] associated with a [`RecordId`], applying the normalizations present in the
/// [`Normalization`].
///
/// The database state is passed back to the caller and must be commited for the record to be
/// recorded in the database.
pub fn get_record_row<'conn>(
    db: &'conn mut RecordDatabase,
    record_id: RecordId,
    client: &HttpClient,
    normalization: &Normalization,
) -> Result<RecordRowResponse<'conn>, Error> {
    match db.state_from_record_id(record_id)? {
        RecordIdState::Existent(key, row) => {
            let RowData {
                data, canonical, ..
            } = row.get_data()?;
            Ok(RecordRowResponse::Exists(
                Record {
                    key,
                    data,
                    canonical,
                },
                row,
            ))
        }
        RecordIdState::NullRemoteId(remote_id, null_row) => {
            Ok(RecordRowResponse::NullRemoteId(remote_id.key, null_row))
        }
        RecordIdState::UndefinedAlias(alias) => Ok(RecordRowResponse::NullAlias(alias)),
        RecordIdState::InvalidRemoteId(err) => Ok(RecordRowResponse::InvalidRemoteId(err)),
        RecordIdState::UnknownRemoteId(maybe_normalized, missing) => {
            get_record_row_recursive(missing, maybe_normalized, client, normalization)
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
/// At each intermediate stage, attempt to read any data possible from the database
/// inside the transaction implicit in the [`State<Missing>`], and write any new data to the
/// database.
fn get_record_row_recursive<'conn>(
    mut missing: State<'conn, Missing>,
    mapped_remote_id: MappedKey<RemoteId>,
    client: &HttpClient,
    normalization: &Normalization,
) -> Result<RecordRowResponse<'conn>, Error> {
    info!("Resolving remote record for {}", mapped_remote_id);
    let mut history = NonEmpty::singleton(mapped_remote_id.key);
    loop {
        missing = match get_remote_response(client, history.last())? {
            RemoteResponse::Data(mut data) => {
                data.normalize(normalization);
                let raw_record_data = (&data).into();
                let row = missing.insert(&raw_record_data, history.last())?;
                row.add_refs(history.iter())?;

                let NonEmpty { head, mut tail } = history;
                let (key, canonical) = match (mapped_remote_id.original, tail.pop()) {
                    (Some(key), Some(canonical)) => (key, canonical),
                    (Some(key), None) => (key, head),
                    (None, Some(canonical)) => (head.into(), canonical),
                    (None, None) => (head.to_string(), head),
                };

                break Ok(RecordRowResponse::Exists(
                    Record {
                        key,
                        data: RawRecordData::from(&data),
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
                    break Ok(RecordRowResponse::Exists(
                        Record {
                            key: mapped_remote_id.original.unwrap_or(history.head.into()),
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
                    break Ok(RecordRowResponse::NullRemoteId(remote_id, null_row));
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
    Exists(RecordData, RemoteId),
    /// The remote record does not exist.
    Null(RemoteId),
}

/// Get the [`Record`] associated with a [`RemoteId`], or [`None`] if the [`Record`] does not exist.
///
/// This method does not involve any database reads or writes, and simply loops to obtain the
/// remote record associated with a [`RemoteId`].
pub fn get_remote_response_recursive(
    remote_id: RemoteId,
    client: &HttpClient,
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
