//! # Error implementation
//! The main error types which result from normal usage.
mod database;
mod record;
mod record_data;
mod source;

use thiserror::Error;

use crate::record::{Alias, RemoteId};

pub use database::DatabaseError;
pub use record::{RecordError, RecordErrorKind};
pub use record_data::RecordDataError;
pub use source::SourceError;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    BadRemoteId(#[from] RecordError),
    #[error("Null alias `{0}`")]
    NullAlias(Alias),
    #[error("Null record `{0}`")]
    NullRemoteId(RemoteId),
    #[error("File type `{0}` not supported")]
    UnsupportedFileType(String),
    #[error("File type required")]
    MissingFileType,
    #[error("Database error: {0}")]
    DatabaseError(#[from] DatabaseError),
    #[error("Source error: {0}")]
    SourceError(#[from] SourceError),
}
