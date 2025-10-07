use thiserror::Error;

use crate::entry::{EntryTypeHeader, KeyHeader, ValueHeader};

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
    #[error("Identifier contains invalid character")]
    ContainsInvalidChar,

    #[error(
        "Key has invalid size {0}; must be at least 1 and at most {max}",
        max = KeyHeader::MAX
    )]
    KeyInvalidLength(usize),

    #[error(
        "Entry type has invalid size {0}; must be at least 1 and at most {max}",
        max = EntryTypeHeader::MAX
    )]
    EntryTypeInvalidLength(usize),

    #[error("Value has invalid size {0}; must be at most {max}", max = ValueHeader::MAX)]
    ValueInvalidLength(usize),

    #[error("Value does not contain balanced `{{ }}` braces")]
    ValueNotBalanced,

    #[error("Invalid bytes: `{0}`")]
    InvalidBytes(#[from] InvalidBytesError),
}
