mod key;

pub use key::{Alias, RecordId, RemoteId};

use crate::database::{NullRecordsResponse, RecordDatabase, RecordsResponse};
use crate::entry::KeyedEntry;
use crate::error::Error;
use crate::source::lookup_source;
use crate::HttpClient;

use private::Context;

use either::Either;

/// Get the [`KeyedEntry`] associated with a [`RecordId`], or `None` if the [`KeyedEntry`] does not exist.
pub fn get_record(
    db: &mut RecordDatabase,
    record_id: RecordId,
    client: &HttpClient,
) -> Result<(KeyedEntry, RemoteId), Error> {
    match db.get_cached_data(&record_id)? {
        RecordsResponse::Found(entry, canonical, _) => Ok((entry.add_key(record_id), canonical)),
        RecordsResponse::NotFound => match record_id.resolve()? {
            Either::Left(alias) => Err(Error::NullAlias(alias)),
            Either::Right(remote_id) => remote_resolve(db, Context::new(remote_id), client),
        },
    }
}

/// Loop to resolve remote records.
fn remote_resolve(
    db: &mut RecordDatabase,
    mut context: Context,
    client: &HttpClient,
) -> Result<(KeyedEntry, RemoteId), Error> {
    loop {
        let top = context.peek();
        match db.get_cached_null(top)? {
            NullRecordsResponse::Found(_when) => {
                // skip top element of Context since it is already cached
                db.set_cached_null(context.descend().skip(1))?;
                break Err(Error::NullRemoteId(context.into_top()));
            }
            NullRecordsResponse::NotFound => match lookup_source(top.source()) {
                Either::Left(resolver) => match resolver(top.sub_id(), client)? {
                    Some(entry) => {
                        db.set_cached_data(top, &entry, context.descend())?;
                        let (bottom, top) = context.into_ends();
                        break Ok((entry.add_key(bottom), top));
                    }
                    None => {
                        db.set_cached_null(context.descend())?;
                        break Err(Error::NullRemoteId(context.into_bottom()));
                    }
                },
                Either::Right(referrer) => match referrer(top.sub_id(), client)? {
                    Some(new_remote_id) => {
                        match db.get_cached_data_and_ref(&new_remote_id, context.descend())? {
                            RecordsResponse::Found(entry, canonical, _) => {
                                break Ok((entry.add_key(new_remote_id), canonical))
                            }
                            RecordsResponse::NotFound => context.push(new_remote_id),
                        }
                    }
                    None => {
                        db.set_cached_null(context.descend())?;
                        break Err(Error::NullRemoteId(context.into_bottom()));
                    }
                },
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
        pub fn new(first: RemoteId) -> Self {
            Self(vec![first])
        }

        /// Iterate from top to bottom
        pub fn descend(&self) -> impl Iterator<Item = &RemoteId> {
            self.0.iter().rev()
        }

        /// Push a new element.
        pub fn push(&mut self, remote_id: RemoteId) {
            self.0.push(remote_id)
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
                    let top = self.0.pop().unwrap_unchecked();
                    let bottom = self.0.drain(..).next().unwrap_unchecked();
                    (bottom, top)
                // SAFETY: the internal vec is always non-empty
                } else {
                    let top = self.0.pop().unwrap_unchecked();
                    (top.clone(), top)
                }
            }
        }
    }
}
