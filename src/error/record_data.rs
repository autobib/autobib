use thiserror::Error;

use crate::db::{EntryTypeHeader, KeyHeader, ValueHeader};

#[derive(Error, Debug, PartialEq)]
#[error("Invalid bytes: error at position `{position}`: {message}")]
pub struct InvalidBytesError {
    pub position: usize,
    pub message: &'static str,
}

impl InvalidBytesError {
    pub fn new(position: usize, message: &'static str) -> Self {
        Self { position, message }
    }
}

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

    #[error("Invalid bytes: `{0}`")]
    InvalidBytes(#[from] InvalidBytesError),

    #[error("Record data structure contains maximum number of entries")]
    RecordDataFull,

    #[error("Record data could not be parsed from bibtex string.")]
    BibtexReadError,
}
