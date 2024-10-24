mod key;

use log::info;

pub use self::key::{Alias, RecordId, RemoteId};
use crate::{
    db::{
        state::{
            add_refs, get_row_data, DatabaseState, MissingRow, NullRecordRow, RecordIdState,
            RecordRow, RemoteIdState,
        },
        RawRecordData, RecordData, RecordDatabase, RowData,
    },
    error::{Error, ProviderError, RecordError},
    provider::{get_remote_response, RemoteResponse},
    HttpClient,
};

use private::NonEmptyStack;

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
/// If the record exists, the resulting [`RecordRow`] is guaranteed to be valid for the row corresponding
/// to the [`Record`].
///
/// If the record does not exist, then the resulting [`NullRecordRow`] is guaranteed to not exist in the
/// `Records` table, and be cached in the `NullRecords` table.
///
/// The database state is passed back to the caller inside the enum. Note that this
/// transaction *must* be committed in order for database changes to be in effect, regardless if
/// the record exists or is null, since the null records are also cached inside the database.
#[derive(Debug)]
pub enum RecordRowResponse<'conn> {
    /// The record exists.
    Exists(Record, RecordRow<'conn>),
    /// The record is null.
    NullRemoteId(RemoteId, NullRecordRow<'conn>),
    /// The identifier has an invalid form.
    InvalidRemoteId(RecordError),
    /// The alias does not exist.
    NullAlias(Alias),
}

/// Get the [`Record`] associated with a [`RecordId`].
///
/// The database state is passed back to the caller and must be commited for the record to be
/// recorded in the database.
pub fn get_record_row<'conn>(
    db: &'conn mut RecordDatabase,
    record_id: RecordId,
    client: &HttpClient,
) -> Result<RecordRowResponse<'conn>, Error> {
    match db.state_from_record_id(record_id)? {
        RecordIdState::Existent(record_id, row) => {
            let RowData {
                data, canonical, ..
            } = row.apply(get_row_data)?;
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
            get_record_row_recursive(missing, remote_id, client)
        }
    }
}

/// Resolve remote records inside a loop within a transaction.
///
/// At each intermediate stage, attempt to read any data possible from the database
/// inside the transaction implicit in the [`MissingRow`], and write any new data to the
/// database.
fn get_record_row_recursive<'conn>(
    mut missing: MissingRow<'conn>,
    remote_id: RemoteId,
    client: &HttpClient,
) -> Result<RecordRowResponse<'conn>, Error> {
    info!("Resolving remote record for '{remote_id}'");
    let mut history = NonEmptyStack::new(remote_id);
    loop {
        missing = match get_remote_response(client, history.peek())? {
            RemoteResponse::Data(data) => {
                let raw_record_data = (&data).into();
                let row = missing.insert(&raw_record_data, history.peek())?;
                row.apply(add_refs(history.descend()))?;
                let (bottom, top) = history.into_ends();
                break Ok(RecordRowResponse::Exists(
                    Record {
                        key: bottom.into(),
                        data: RawRecordData::from(&data),
                        canonical: top,
                    },
                    row,
                ));
            }
            RemoteResponse::Reference(new_remote_id) => match missing.reset(&new_remote_id)? {
                RemoteIdState::Existent(row) => {
                    // not necessary to insert `new_remote_id` since we just saw that it
                    // is present in the database
                    row.apply(add_refs(history.descend()))?;
                    let RowData {
                        data, canonical, ..
                    } = row.apply(get_row_data)?;
                    break Ok(RecordRowResponse::Exists(
                        Record {
                            key: history.into_bottom().into(),
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
                if history.is_singleton() {
                    let remote_id = history.into_top();
                    let null_row = missing.set_null(&remote_id)?;
                    break Ok(RecordRowResponse::NullRemoteId(remote_id, null_row));
                } else {
                    break Err(ProviderError::UnexpectedNullRemoteFromProvider(
                        history.into_top().into(),
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
    let mut history = NonEmptyStack::new(remote_id);
    loop {
        let top = history.peek();

        match get_remote_response(client, top)? {
            RemoteResponse::Data(data) => {
                break Ok(RecursiveRemoteResponse::Exists(data, history.into_top()));
            }
            RemoteResponse::Reference(new_remote_id) => {
                history.push(new_remote_id);
            }
            RemoteResponse::Null => {
                break Ok(RecursiveRemoteResponse::Null(history.into_bottom()));
            }
        }
    }
}

mod private {
    /// A non-empty stack implementation.
    #[derive(Debug)]
    pub struct NonEmptyStack<T>(Vec<T>);

    impl<T> NonEmptyStack<T> {
        /// Construct a new [`NonEmptyStack`] with an initial element.
        #[inline]
        pub fn new(first: T) -> Self {
            Self(vec![first])
        }

        /// Check if the stack consists of a single element
        #[inline]
        pub fn is_singleton(&self) -> bool {
            self.0.len() == 1
        }

        /// Iterate from top to bottom
        #[inline]
        pub fn descend(&self) -> impl Iterator<Item = &T> {
            self.0.iter().rev()
        }

        /// Push a new element.
        #[inline]
        pub fn push(&mut self, remote_id: T) {
            self.0.push(remote_id);
        }

        /// Get the top element.
        #[inline]
        pub fn peek(&self) -> &T {
            // SAFETY: the internal vec is always non-empty
            unsafe { self.0.last().unwrap_unchecked() }
        }

        /// Drop the stack, extracting the bottom element.
        #[inline]
        pub fn into_bottom(mut self) -> T {
            // SAFETY: the internal vec is always non-empty
            unsafe { self.0.drain(..).next().unwrap_unchecked() }
        }

        /// Drop the stack, extracting the top element.
        #[inline]
        pub fn into_top(self) -> T {
            self.split_top().1
        }

        /// Drop the stack, extracting the top element.
        #[inline]
        pub fn split_top(mut self) -> (Vec<T>, T) {
            // SAFETY: the internal vec is always non-empty
            let top = unsafe { self.0.pop().unwrap_unchecked() };
            (self.0, top)
        }
    }

    impl<T: Clone> NonEmptyStack<T> {
        /// Drop the stack, extracting the top and bottom elements.
        ///
        /// Note that `T` must be [`Clone`], since it is possible that the stack has exactly one
        /// element, so the top and the bottom elements are the same element.
        #[inline]
        pub fn into_ends(mut self) -> (T, T) {
            unsafe {
                if self.0.len() >= 2 {
                    // SAFETY: we just checked that the length is at least 2
                    let top = self.0.pop().unwrap_unchecked();
                    let bottom = self.0.drain(..).next().unwrap_unchecked();
                    (bottom, top)
                } else {
                    // SAFETY: the internal vec is always non-empty
                    let top = self.0.pop().unwrap_unchecked();
                    (top.clone(), top)
                }
            }
        }
    }
}
