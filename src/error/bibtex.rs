use thiserror::Error;

use super::RecordDataError;

#[derive(Error, Debug)]
pub enum BibTeXError {
    #[error("Invalid record data: {0}")]
    InvalidData(#[from] RecordDataError),
    #[error("Invalid BibTeX entry key: {0}")]
    InvalidKey(String),
    #[error("Entry could not be parsed from BibTeX string.")]
    BibtexParseError,
    #[error("BibTeX string contained multiple entries.")]
    BibtexMultipleEntries,
}
