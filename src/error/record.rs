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
    EmptyAlias,
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid citation key '{}': ", self.input)?;
        match self.kind {
            RecordErrorKind::EmptyProvider => f.write_str("provider must be non-empty"),
            RecordErrorKind::EmptySubId => f.write_str("sub-id must be non-empty"),
            RecordErrorKind::EmptyAlias => f.write_str("alias must be non-empty"),
            RecordErrorKind::InvalidProvider => f.write_str("provider is invalid"),
            RecordErrorKind::InvalidSubId => {
                f.write_str("sub-id is invalid for the given provider")
            }
            RecordErrorKind::RecordIdIsNotAlias => f.write_str("alias must not contain a colon"),
        }
    }
}
