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
    database::DatabaseError,
    provider::ProviderError,
    record::{
        AliasConversionError, AliasErrorKind, RecordError, RecordErrorKind,
        RemoteIdConversionError, RemoteIdErrorKind,
    },
    record_data::{InvalidBytesError, RecordDataError},
};

#[derive(Error, Debug)]
pub enum MergeError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] DatabaseError),
    #[error("Error adding data: {0}")]
    RecordError(#[from] RecordDataError),
}

/// A trait for errors which have a representation which only depends on the variant, and not on
/// particular data associated with the error.
pub trait ShortError {
    /// Represent an error in short form.
    fn short_err(&self) -> &'static str;
}

#[derive(Error, Debug)]
pub enum Error {
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
