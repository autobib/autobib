mod database;
mod record;
mod source;

use std::fmt;

use crate::record::{Alias, RemoteId};

pub use database::DatabaseError;
pub use record::{RecordError, RecordErrorKind};
pub use source::SourceError;

#[derive(Debug)]
pub enum Error {
    BadRemoteId(RecordError),
    NullAlias(Alias),
    NullRemoteId(RemoteId),
    DatabaseError(DatabaseError),
    SourceError(SourceError),
}

impl From<RecordError> for Error {
    fn from(err: RecordError) -> Self {
        Self::BadRemoteId(err)
    }
}

impl From<DatabaseError> for Error {
    fn from(err: DatabaseError) -> Self {
        Self::DatabaseError(err)
    }
}

impl From<SourceError> for Error {
    fn from(err: SourceError) -> Self {
        Self::SourceError(err)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NullAlias(alias) => write!(f, "Null alias '{alias}'"),
            Error::NullRemoteId(error) => write!(f, "Null record '{error}'"),
            Error::BadRemoteId(error) => write!(f, "Key error: {error}"),
            Error::DatabaseError(error) => write!(f, "Database error: {error}'"),
            Error::SourceError(error) => write!(f, "Source error: {error}"),
        }
    }
}

impl std::error::Error for Error {}
