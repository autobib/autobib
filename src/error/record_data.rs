use thiserror::Error;

use crate::db::{EntryTypeHeader, KeyHeader, ValueHeader};

#[derive(Error, Debug, PartialEq)]
pub enum RecordDataError {
    #[error("Key is not ASCII lowercase `[a-z]`")]
    KeyNotAsciiLowercase,

    #[error(
        "Key has invalid size {0}; must be at least 1 and at most {}",
        KeyHeader::MAX
    )]
    KeyInvalidLength(usize),

    #[error("Entry type is not ASCII lowercase `[a-z]`")]
    EntryTypeNotAsciiLowercase,

    #[error(
        "Entry type has invalid size {0}; must be at least 1 and at most {}",
        EntryTypeHeader::MAX
    )]
    EntryTypeInvalidLength(usize),

    #[error("Value has invalid size {0}; must be at most {}", ValueHeader::MAX)]
    ValueInvalidLength(usize),

    #[error("Value does not contain balanced `{{ }}` brackets")]
    ValueNotBalanced,

    #[error("Record data structure contains maximum number of entries")]
    RecordDataFull,

    #[error("Record data could not be parsed from bibtex string.")]
    BibtexReadError,
}
