use thiserror::Error;

use super::RecordDataError;

#[derive(Error, Debug)]
pub enum BibtexDataError {
    #[error("Invalid record data: {0}")]
    InvalidData(#[from] RecordDataError),
    #[error("Entry could not be parsed from BibTeX string.")]
    BibtexParseError,
    #[error("BibTeX string contained multiple entries.")]
    BibtexMultipleEntries,
}
