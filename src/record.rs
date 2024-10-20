mod key;

use either::Either;
use log::info;

pub use key::{Alias, RecordId, RemoteId};

use crate::{
    db::{
        row::{
            add_refs, check_null, get_row_data, set_null, DatabaseEntry, MissingRecordRow,
            NullRecordsResponse, RecordRow,
        },
        RawRecordData, RecordDatabase, RowData,
    },
    error::Error,
    provider::lookup_provider,
    HttpClient,
};

use private::NonEmptyStack;

/// The funamdental record type.
#[derive(Debug)]
pub struct Record {
    /// The original key.
    pub key: String,
    /// The raw data.
    pub data: RawRecordData,
    /// The canonical identifier.
    pub canonical: RemoteId,
}

/// The response type of [`get_record`].
#[derive(Debug)]
pub enum GetRecordResponse {
    /// The record exists.
    Exists(Record),
    /// The remote id corresponding to the record does not exist.
    NullRemoteId(RemoteId),
    /// The alias does not exist in the database.
    NullAlias(Alias),
}

/// The response type of [`get_record_entry`].
///
/// If the record exists, the resulting [`RecordRow`] is guaranteed to be valid for the row corresponding
/// to the [`Record`].
///
/// If the record does not exist, then the corresponding row is guaranteed to not exist.
///
/// The initial [`DatabaseEntry`] is passed back to the caller inside the enum. Note that this
/// transaction *must* be committed in order for database changes to be in effect, regardless if
/// the record exists or is null, since the null records are also cached inside the database.
///
/// This type can be converted to a usual [`GetRecordResponse`], at which point the internal
/// transaction is automatically committed.
#[derive(Debug)]
pub enum GetRecordEntryResponse<'conn> {
    /// The record exists.
    Exists(Record, RecordRow<'conn>),
    /// The remote id corresponding to the record does not exist.
    NullRemoteId(RemoteId, MissingRecordRow<'conn>),
    /// The alias does not exist in the database.
    NullAlias(Alias, MissingRecordRow<'conn>),
}

impl TryFrom<GetRecordEntryResponse<'_>> for GetRecordResponse {
    type Error = rusqlite::Error;

    fn try_from(res: GetRecordEntryResponse<'_>) -> Result<Self, Self::Error> {
        match res {
            GetRecordEntryResponse::Exists(record, row) => {
                row.commit()?;
                Ok(GetRecordResponse::Exists(record))
            }
            GetRecordEntryResponse::NullRemoteId(remote_id, missing) => {
                missing.commit()?;
                Ok(GetRecordResponse::NullRemoteId(remote_id))
            }
            GetRecordEntryResponse::NullAlias(alias, missing) => {
                missing.commit()?;
                Ok(GetRecordResponse::NullAlias(alias))
            }
        }
    }
}

/// Get the [`Record`] associated with a [`RecordId`], or [`None`] if the [`Record`] does not exist.
///
/// This is essentially a convenience method for the [`get_record_entry`] function, except the
/// transaction is created and committed internally.
pub fn get_record(
    db: &mut RecordDatabase,
    record_id: RecordId,
    client: &HttpClient,
) -> Result<GetRecordResponse, Error> {
    Ok(get_record_entry(db.entry(&record_id)?, record_id, client)?.try_into()?)
}

/// Get the [`Record`] associated with a [`RecordId`], except within a [`DatabaseEntry`].
///
/// The [`DatabaseEntry`] is passed back to the caller and must be commited for the record to be
/// recorded in the database.
pub fn get_record_entry<'conn>(
    entry: DatabaseEntry<'conn>,
    record_id: RecordId,
    client: &HttpClient,
) -> Result<GetRecordEntryResponse<'conn>, Error> {
    match entry {
        DatabaseEntry::Exists(row) => {
            let RowData {
                data, canonical, ..
            } = row.apply(get_row_data)?;
            Ok(GetRecordEntryResponse::Exists(
                Record {
                    key: record_id.into(),
                    data,
                    canonical,
                },
                row,
            ))
        }
        DatabaseEntry::Missing(missing) => match record_id.resolve()? {
            Either::Left(alias) => Ok(GetRecordEntryResponse::NullAlias(alias, missing)),
            Either::Right(remote_id) => get_record_entry_remote_resolve(missing, remote_id, client),
        },
    }
}

/// Resolve remote records inside a loop within a transaction.
///
/// At each intermediate stage, attempt to read any data possible from the database
/// inside the transaction implicit in the [`MissingRecordRow`], and write any new data to the
/// database.
fn get_record_entry_remote_resolve<'conn>(
    mut missing: MissingRecordRow<'conn>,
    remote_id: RemoteId,
    client: &HttpClient,
) -> Result<GetRecordEntryResponse<'conn>, Error> {
    let mut history = NonEmptyStack::new(remote_id);
    loop {
        let top = history.peek();

        missing = match missing.apply(check_null(top))? {
            NullRecordsResponse::Found(_when) => {
                // skip top element of the stack since it is already cached
                missing.apply(set_null(history.descend().skip(1)))?;
                break Ok(GetRecordEntryResponse::NullRemoteId(
                    history.into_top(),
                    missing,
                ));
            }
            NullRecordsResponse::NotFound => {
                info!("Resolving remote record for '{top}'");
                match lookup_provider(top.provider()) {
                    Either::Left(resolver) => match resolver(top.sub_id(), client)? {
                        Some(data) => {
                            let raw_record_data = (&data).into();
                            let row = missing.insert(&raw_record_data, top)?;
                            row.apply(add_refs(history.descend()))?;
                            let (bottom, top) = history.into_ends();
                            break Ok(GetRecordEntryResponse::Exists(
                                Record {
                                    key: bottom.into(),
                                    data: RawRecordData::from(&data),
                                    canonical: top,
                                },
                                row,
                            ));
                        }
                        None => {
                            missing.apply(set_null(history.descend()))?;
                            break Ok(GetRecordEntryResponse::NullRemoteId(
                                history.into_bottom(),
                                missing,
                            ));
                        }
                    },
                    Either::Right(referrer) => match referrer(top.sub_id(), client)? {
                        Some(new_remote_id) => match missing.reset(&new_remote_id)? {
                            DatabaseEntry::Exists(row) => {
                                row.apply(add_refs(history.descend()))?;
                                let RowData {
                                    data, canonical, ..
                                } = row.apply(get_row_data)?;
                                break Ok(GetRecordEntryResponse::Exists(
                                    Record {
                                        key: history.into_bottom().into(),
                                        data,
                                        canonical,
                                    },
                                    row,
                                ));
                            }
                            DatabaseEntry::Missing(missing) => {
                                history.push(new_remote_id);
                                missing
                            }
                        },
                        None => {
                            missing.apply(set_null(history.descend()))?;
                            break Ok(GetRecordEntryResponse::NullRemoteId(
                                history.into_bottom(),
                                missing,
                            ));
                        }
                    },
                }
            }
        };
    }
}

/// The result of obtaining a remote record, with no reference to a database.
pub enum GetRemoteRecordResponse {
    /// The remote record exists, and has the provided data.
    Exists(RawRecordData),
    /// The remote record does not exist.
    Null(RemoteId),
}

/// Get the [`Record`] associated with a [`RemoteId`], or [`None`] if the [`Record`] does not exist.
///
/// This method does not involve any database reads or writes, and simply loops to obtain the
/// remote record associated with a [`RemoteId`].
pub fn get_remote_record(
    remote_id: RemoteId,
    client: &HttpClient,
) -> Result<GetRemoteRecordResponse, Error> {
    let mut history = NonEmptyStack::new(remote_id);
    loop {
        let top = history.peek();

        info!("Resolving remote record for '{top}'");
        match lookup_provider(top.provider()) {
            Either::Left(resolver) => match resolver(top.sub_id(), client)? {
                Some(data) => {
                    break Ok(GetRemoteRecordResponse::Exists(RawRecordData::from(&data)));
                }
                None => {
                    break Ok(GetRemoteRecordResponse::Null(history.into_bottom()));
                }
            },
            Either::Right(referrer) => match referrer(top.sub_id(), client)? {
                Some(new_remote_id) => history.push(new_remote_id),
                None => {
                    break Ok(GetRemoteRecordResponse::Null(history.into_bottom()));
                }
            },
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
