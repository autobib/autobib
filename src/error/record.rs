use std::fmt;

use thiserror::Error;

use super::ShortError;

#[derive(Error, Debug)]
pub struct RecordError {
    pub input: String,
    pub kind: RecordErrorKind,
}

#[derive(Debug)]
pub enum RecordErrorKind {
    EmptyProvider,
    EmptySubId,
    InvalidProvider,
    InvalidSubId,
    RecordIdIsNotAlias,
    RecordIdIsNotRemoteId,
    EmptyAlias,
}

impl ShortError for RecordError {
    fn short_err(&self) -> &'static str {
        match self.kind {
            RecordErrorKind::EmptyProvider => "provider must contain non-whitespace characters",
            RecordErrorKind::EmptySubId => "sub-id must contain non-whitespace characters",
            RecordErrorKind::InvalidProvider => "provider is invalid",
            RecordErrorKind::InvalidSubId => "sub-id is invalid for the given provider",
            RecordErrorKind::RecordIdIsNotAlias => "alias must not contain a colon",
            RecordErrorKind::RecordIdIsNotRemoteId => "remote id must contain a colon",
            RecordErrorKind::EmptyAlias => "alias must contain non-whitespace characters",
        }
    }
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid key '{}': {}", self.input, self.short_err())
    }
}

#[derive(Error, Debug)]
pub enum AliasConversionError {
    #[error("Invalid alias '{0}': {se}", se = self.short_err())]
    IsRemoteId(String),
    #[error("Invalid alias '{0}': {se}", se = self.short_err())]
    Empty(String),
}

impl ShortError for AliasConversionError {
    fn short_err(&self) -> &'static str {
        match self {
            AliasConversionError::IsRemoteId(_) => "alias must not contain a colon",
            AliasConversionError::Empty(_) => "alias must contain non-whitespace characters",
        }
    }
}

impl From<AliasConversionError> for RecordError {
    fn from(err: AliasConversionError) -> Self {
        match err {
            AliasConversionError::IsRemoteId(input) => RecordError {
                input,
                kind: RecordErrorKind::RecordIdIsNotAlias,
            },
            AliasConversionError::Empty(input) => RecordError {
                input,
                kind: RecordErrorKind::EmptyAlias,
            },
        }
    }
}
