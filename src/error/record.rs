use std::fmt;

use thiserror::Error;

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

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid key '{}': ", self.input)?;
        match self.kind {
            RecordErrorKind::EmptyProvider => {
                f.write_str("provider must contain non-whitespace characters")
            }
            RecordErrorKind::EmptySubId => {
                f.write_str("sub-id must contain non-whitespace characters")
            }
            RecordErrorKind::EmptyAlias => {
                f.write_str("alias must contain non-whitespace characters")
            }
            RecordErrorKind::InvalidProvider => f.write_str("provider is invalid"),
            RecordErrorKind::InvalidSubId => {
                f.write_str("sub-id is invalid for the given provider")
            }
            RecordErrorKind::RecordIdIsNotAlias => f.write_str("alias must not contain a colon"),
            RecordErrorKind::RecordIdIsNotRemoteId => f.write_str("remote id must contain a colon"),
        }
    }
}

#[derive(Error, Debug)]
pub enum AliasConversionError {
    #[error("Invalid alias '{0}': alias must not contain a colon")]
    IsRemoteId(String),
    #[error("Invalid alias '{0}': alias must contain non-whitespace characters")]
    Empty(String),
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
