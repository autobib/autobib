//! # Error implementation
//! The main error types which result from normal usage.
mod bibtex;
mod database;
mod format;
mod provider;
mod record;
mod record_data;

use std::{error, fmt, ops::Range};

use crossterm::style::Stylize;
use mufmt::SyntaxError;
use thiserror::Error;

pub use self::{
    bibtex::BibtexDataError,
    database::DatabaseError,
    format::{KeyParseError, KeyParseErrorKind},
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

/// A special error wrapper type which has displays a template syntax error nicely for clap.
#[derive(Debug)]
pub struct ClapTemplateError(pub SyntaxError<KeyParseError>);

impl fmt::Display for ClapTemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const CLAP_PREFIX_LEN: usize = "error: invalid value '".len();

        let Range { start, end } = self.0.locate();

        let (start, end) = if let mufmt::SyntaxErrorKind::InvalidExpr(ref e) = self.0.kind
            && let Some(Range { start: l, end: r }) = e.span
        {
            (start + l, start + r)
        } else {
            (start, end)
        };

        let loc = format!("{caret:^>0$}", end - start, caret = "^")
            .stylize()
            .red();
        write!(
            f,
            "\n{space:>0$}{loc}\n{help} {kind}",
            CLAP_PREFIX_LEN + start,
            space = " ",
            help = "help:".stylize().blue().bold(),
            kind = self.0.kind,
        )
    }
}

impl error::Error for ClapTemplateError {}
