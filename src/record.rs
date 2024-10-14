mod key;

use either::Either;
use log::info;

pub use key::{Alias, RecordId, RemoteId};

use crate::{
    db::{
        row::{
            add_refs, check_null, get_row_data, set_null, DatabaseEntry, Missing,
            NullRecordsResponse, Row,
        },
        RawRecordData, RecordDatabase, RowData,
    },
    error::Error,
    provider::lookup_provider,
    HttpClient,
};

use private::Context;

#[derive(Debug)]
pub struct Record {
    pub key: String,
    pub data: RawRecordData,
    pub canonical: RemoteId,
}

#[derive(Debug)]
pub enum GetRecordResponse {
    /// The record exists.
    Exists(Record),
    /// The remote id corresponding to the record does not exist.
    NullRemoteId(RemoteId),
    /// The alias does not exist in the database.
    NullAlias(Alias),
}

impl GetRecordResponse {
    /// Return `Some(Record)` if the record exists, and otherwise `None`.
    pub fn ok(self) -> Option<Record> {
        match self {
            Self::Exists(record) => Some(record),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum GetRecordEntryResponse<'conn> {
    /// The record exists.
    Exists(Record, Row<'conn>),
    /// The remote id corresponding to the record does not exist.
    NullRemoteId(RemoteId, Missing<'conn>),
    /// The alias does not exist in the database.
    NullAlias(Alias, Missing<'conn>),
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
            Either::Right(remote_id) => remote_resolve(missing, Context::new(remote_id), client),
        },
    }
}

/// Loop to resolve remote records.
fn remote_resolve<'conn>(
    mut missing: Missing<'conn>,
    mut context: Context,
    client: &HttpClient,
) -> Result<GetRecordEntryResponse<'conn>, Error> {
    loop {
        let top = context.peek();

        missing = match missing.apply(check_null(top))? {
            NullRecordsResponse::Found(_when) => {
                // skip top element of Context since it is already cached
                missing.apply(set_null(context.descend().skip(1)))?;
                break Ok(GetRecordEntryResponse::NullRemoteId(
                    context.into_top(),
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
                            row.apply(add_refs(context.descend()))?;
                            let (bottom, top) = context.into_ends();
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
                            missing.apply(set_null(context.descend()))?;
                            break Ok(GetRecordEntryResponse::NullRemoteId(
                                context.into_bottom(),
                                missing,
                            ));
                        }
                    },
                    Either::Right(referrer) => match referrer(top.sub_id(), client)? {
                        Some(new_remote_id) => match missing.reset(&new_remote_id)? {
                            DatabaseEntry::Exists(row) => {
                                row.apply(add_refs(context.descend()))?;
                                let RowData {
                                    data, canonical, ..
                                } = row.apply(get_row_data)?;
                                break Ok(GetRecordEntryResponse::Exists(
                                    Record {
                                        key: context.into_bottom().into(),
                                        data,
                                        canonical,
                                    },
                                    row,
                                ));
                            }
                            DatabaseEntry::Missing(missing) => {
                                context.push(new_remote_id);
                                missing
                            }
                        },
                        None => {
                            missing.apply(set_null(context.descend()))?;
                            break Ok(GetRecordEntryResponse::NullRemoteId(
                                context.into_bottom(),
                                missing,
                            ));
                        }
                    },
                }
            }
        };
    }
}

pub enum GetRemoteRecordResponse {
    Exists(RawRecordData),
    Null(RemoteId),
}

/// Get the [`Record`] associated with a [`RemoteId`], or [`None`] if the [`Record`] does not exist.
pub fn get_remote_record(
    remote_id: RemoteId,
    client: &HttpClient,
) -> Result<GetRemoteRecordResponse, Error> {
    let mut context = Context::new(remote_id);
    loop {
        let top = context.peek();

        info!("Resolving remote record for '{top}'");
        match lookup_provider(top.provider()) {
            Either::Left(resolver) => match resolver(top.sub_id(), client)? {
                Some(data) => {
                    break Ok(GetRemoteRecordResponse::Exists(RawRecordData::from(&data)));
                }
                None => {
                    break Ok(GetRemoteRecordResponse::Null(context.into_bottom()));
                }
            },
            Either::Right(referrer) => match referrer(top.sub_id(), client)? {
                Some(new_remote_id) => context.push(new_remote_id),
                None => {
                    break Ok(GetRemoteRecordResponse::Null(context.into_bottom()));
                }
            },
        }
    }
}

mod private {
    use super::RemoteId;

    /// A [`Context`] is a non-empty stack holding [`RemoteId`].
    #[derive(Debug)]
    pub struct Context(Vec<RemoteId>);

    impl Context {
        /// Construct a new [`Context`] with an initial element.
        #[inline]
        pub fn new(first: RemoteId) -> Self {
            Self(vec![first])
        }

        /// Iterate from top to bottom
        #[inline]
        pub fn descend(&self) -> impl Iterator<Item = &RemoteId> {
            self.0.iter().rev()
        }

        /// Push a new element.
        #[inline]
        pub fn push(&mut self, remote_id: RemoteId) {
            self.0.push(remote_id);
        }

        /// Get the top element.
        #[inline]
        pub fn peek(&self) -> &RemoteId {
            // SAFETY: the internal vec is always non-empty
            unsafe { self.0.last().unwrap_unchecked() }
        }

        /// Drop the context, extracting the bottom element.
        #[inline]
        pub fn into_bottom(mut self) -> RemoteId {
            // SAFETY: the internal vec is always non-empty
            unsafe { self.0.drain(..).next().unwrap_unchecked() }
        }

        /// Drop the context, extracting the top element.
        #[inline]
        pub fn into_top(self) -> RemoteId {
            self.split_top().1
        }

        /// Drop the context, extracting the top element.
        #[inline]
        pub fn split_top(mut self) -> (Vec<RemoteId>, RemoteId) {
            // SAFETY: the internal vec is always non-empty
            let top = unsafe { self.0.pop().unwrap_unchecked() };
            (self.0, top)
        }

        /// Drop the context, extracting the top and bottom elements.
        #[inline]
        pub fn into_ends(mut self) -> (RemoteId, RemoteId) {
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
