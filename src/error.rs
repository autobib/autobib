//! # Error implementation
//! The main error types which result from normal usage.
mod bibtex;
mod database;
mod provider;
mod record;
mod record_data;

use thiserror::Error;

pub use self::{
    bibtex::BibtexDataError,
    database::{DatabaseError, ValidationError},
    provider::ProviderError,
    record::{RecordError, RecordErrorKind},
    record_data::{InvalidBytesError, RecordDataError},
};

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    BadRemoteId(#[from] RecordError),
    #[error("File type '{0}' not supported")]
    UnsupportedFileType(String),
    #[error("File type required")]
    MissingFileType,
    #[error("Database error: {0}")]
    DatabaseError(#[from] DatabaseError),
    #[error("Provider error: {0}")]
    ProviderError(#[from] ProviderError),
}

impl From<rusqlite::Error> for Error {
    fn from(value: rusqlite::Error) -> Self {
        Self::DatabaseError(value.into())
    }
}
