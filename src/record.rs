mod key;

use either::Either;
use log::info;

pub use key::{Alias, RecordId, RemoteId};

use crate::{
    db::{NullRecordsResponse, RawRecordData, RecordDatabase, RecordsResponse},
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

/// Get the [`Record`] associated with a [`RecordId`], or [`None`] if the [`Record`] does not exist.
pub fn get_record(
    db: &mut RecordDatabase,
    record_id: RecordId,
    client: &HttpClient,
) -> Result<GetRecordResponse, Error> {
    match db.get_cached_data(&record_id)? {
        RecordsResponse::Found(raw_data, canonical, _) => {
            info!("Found cached record for '{record_id}'");
            Ok(GetRecordResponse::Exists(Record {
                key: record_id.into(),
                data: raw_data,
                canonical,
            }))
        }
        RecordsResponse::NotFound => match record_id.resolve()? {
            Either::Left(alias) => Ok(GetRecordResponse::NullAlias(alias)),
            Either::Right(remote_id) => remote_resolve(db, Context::new(remote_id), client),
        },
    }
}

/// Loop to resolve remote records.
fn remote_resolve(
    db: &mut RecordDatabase,
    mut context: Context,
    client: &HttpClient,
) -> Result<GetRecordResponse, Error> {
    loop {
        let top = context.peek();

        match db.get_cached_null(top)? {
            NullRecordsResponse::Found(_when) => {
                // skip top element of Context since it is already cached
                db.set_cached_null(context.descend().skip(1))?;
                break Ok(GetRecordResponse::NullRemoteId(context.into_top()));
            }
            NullRecordsResponse::NotFound => {
                info!("Resolving remote record for '{top}'");
                match lookup_provider(top.provider()) {
                    Either::Left(resolver) => match resolver(top.sub_id(), client)? {
                        Some(data) => {
                            let raw_record_data = (&data).into();
                            db.set_cached_data(top, &raw_record_data, context.descend())?;
                            let (bottom, top) = context.into_ends();
                            break Ok(GetRecordResponse::Exists(Record {
                                key: bottom.into(),
                                data: RawRecordData::from(&data),
                                canonical: top,
                            }));
                        }
                        None => {
                            db.set_cached_null(context.descend())?;
                            break Ok(GetRecordResponse::NullRemoteId(context.into_bottom()));
                        }
                    },
                    Either::Right(referrer) => match referrer(top.sub_id(), client)? {
                        Some(new_remote_id) => {
                            match db.get_cached_data_and_ref(&new_remote_id, context.descend())? {
                                RecordsResponse::Found(raw_data, canonical, _) => {
                                    break Ok(GetRecordResponse::Exists(Record {
                                        key: context.into_bottom().into(),
                                        data: raw_data,
                                        canonical,
                                    }))
                                }
                                RecordsResponse::NotFound => context.push(new_remote_id),
                            }
                        }
                        None => {
                            db.set_cached_null(context.descend())?;
                            break Ok(GetRecordResponse::NullRemoteId(context.into_bottom()));
                        }
                    },
                }
            }
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
        pub fn new(first: RemoteId) -> Self {
            Self(vec![first])
        }

        /// Iterate from top to bottom
        pub fn descend(&self) -> impl Iterator<Item = &RemoteId> {
            self.0.iter().rev()
        }

        /// Push a new element.
        pub fn push(&mut self, remote_id: RemoteId) {
            self.0.push(remote_id);
        }

        /// Get the top element.
        pub fn peek(&self) -> &RemoteId {
            // SAFETY: the internal vec is always non-empty
            unsafe { self.0.last().unwrap_unchecked() }
        }

        /// Drop the context, extracting the bottom element.
        pub fn into_bottom(mut self) -> RemoteId {
            // SAFETY: the internal vec is always non-empty
            unsafe { self.0.drain(..).next().unwrap_unchecked() }
        }

        /// Drop the context, extracting the top element.
        pub fn into_top(mut self) -> RemoteId {
            // SAFETY: the internal vec is always non-empty
            unsafe { self.0.pop().unwrap_unchecked() }
        }

        /// Drop the context, extracting the top and bottom elements.
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
