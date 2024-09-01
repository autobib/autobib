//! # Error implementation
//! The main error types which result from normal usage.
mod database;
mod provider;
mod record;
mod record_data;

use thiserror::Error;

use crate::record::{Alias, RemoteId};

pub use self::{
    database::DatabaseError,
    provider::ProviderError,
    record::{RecordError, RecordErrorKind},
    record_data::RecordDataError,
};

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    BadRemoteId(#[from] RecordError),
    #[error("Null alias '{0}'")]
    NullAlias(Alias),
    #[error("Null record '{0}'")]
    NullRemoteId(RemoteId),
    #[error("File type '{0}' not supported")]
    UnsupportedFileType(String),
    #[error("File type required")]
    MissingFileType,
    #[error("Database error: {0}")]
    DatabaseError(#[from] DatabaseError),
    #[error("Provider error: {0}")]
    ProviderError(#[from] ProviderError),
}
