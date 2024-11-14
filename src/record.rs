mod key;

use anyhow::bail;
use log::info;
use nonempty::NonEmpty;

pub use self::key::{Alias, AliasOrRemoteId, RecordId, RemoteId};
use crate::{
    db::{
        state::{Missing, NullRecordRow, RecordIdState, RecordRow, RemoteIdState, RowData, State},
        RawRecordData, RecordData, RecordDatabase,
    },
    error::{Error, ProviderError, RecordError},
    normalize::Normalize,
    provider::{get_remote_response, RemoteResponse},
    Config, HttpClient,
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

/// Get the [`Record`] associated with a [`RecordId`].
///
/// The database state is passed back to the caller and must be commited for the record to be
/// recorded in the database.
pub fn get_record_row<'conn>(
    db: &'conn mut RecordDatabase,
    record_id: RecordId,
    client: &HttpClient,
    config: &Config,
) -> Result<RecordRowResponse<'conn>, Error> {
    match db.state_from_record_id(record_id)? {
        RecordIdState::Existent(record_id, row) => {
            let RowData {
                data, canonical, ..
            } = row.get_data()?;
            Ok(RecordRowResponse::Exists(
                Record {
                    key: record_id.into(),
                    data,
                    canonical,
                },
                row,
            ))
        }
        RecordIdState::NullRemoteId(remote_id, null_row) => {
            Ok(RecordRowResponse::NullRemoteId(remote_id, null_row))
        }
        RecordIdState::UndefinedAlias(alias) => Ok(RecordRowResponse::NullAlias(alias)),
        RecordIdState::InvalidRemoteId(err) => Ok(RecordRowResponse::InvalidRemoteId(err)),
        RecordIdState::UnknownRemoteId(remote_id, missing) => {
            get_record_row_recursive(missing, remote_id, client, config)
        }
    }
}

/// Destructure a [`NonEmpty`] and return the last element.
#[inline]
fn into_last<T>(ne: NonEmpty<T>) -> T {
    let NonEmpty { head, mut tail } = ne;
    tail.pop().unwrap_or(head)
}

/// Destructure a [`NonEmpty`] and return the first and last elements. If the [`NonEmpty`] has
/// length exactly 1, then this will clone the unique element.
#[inline]
fn into_ends<T: Clone>(ne: NonEmpty<T>) -> (T, T) {
    let NonEmpty { head, mut tail } = ne;
    // destructure to avoid borrow issues
    match tail.pop() {
        Some(last) => (head, last),
        None => (head.clone(), head),
    }
}

/// Resolve remote records inside a loop within a transaction.
///
/// At each intermediate stage, attempt to read any data possible from the database
/// inside the transaction implicit in the [`State<Missing>`], and write any new data to the
/// database.
fn get_record_row_recursive<'conn>(
    mut missing: State<'conn, Missing>,
    remote_id: RemoteId,
    client: &HttpClient,
    config: &Config,
) -> Result<RecordRowResponse<'conn>, Error> {
    info!("Resolving remote record for '{remote_id}'");
    let mut history = NonEmpty::singleton(remote_id);
    loop {
        missing = match get_remote_response(client, history.last())? {
            RemoteResponse::Data(mut data) => {
                data.normalize(&config.on_insert);
                let raw_record_data = (&data).into();
                let row = missing.insert(&raw_record_data, history.last())?;
                row.add_refs(history.iter())?;

                // extract bottom and top simultaneously
                let (first, last) = into_ends(history);

                break Ok(RecordRowResponse::Exists(
                    Record {
                        key: first.into(),
                        data: RawRecordData::from(&data),
                        canonical: last,
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
                            key: history.head.into(),
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
