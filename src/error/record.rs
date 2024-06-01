use std::fmt;

use thiserror::Error;

#[derive(Error, Debug)]
pub struct RecordError {
    pub input: String,
    pub kind: RecordErrorKind,
}

#[derive(Debug)]
pub enum RecordErrorKind {
    EmptySource,
    EmptySubId,
    InvalidSource,
    InvalidSubId,
    RecordIdIsNotAlias,
    EmptyAlias,
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid citation key `{}`: ", self.input)?;
        match self.kind {
            RecordErrorKind::EmptySource => f.write_str("'source' must be non-empty"),
            RecordErrorKind::EmptySubId => f.write_str("'sub_id' must be non-empty"),
            RecordErrorKind::EmptyAlias => f.write_str("alias must be non-empty"),
            RecordErrorKind::InvalidSource => f.write_str("'source' is invalid"),
            RecordErrorKind::InvalidSubId => {
                f.write_str("sub-id is invalid for the provided source")
            }
            RecordErrorKind::RecordIdIsNotAlias => f.write_str("alias must not contain a colon"),
        }
    }
}
