pub use biblatex::Entry;
pub use chrono::{DateTime, Local};
// use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::fmt;
use std::str::FromStr;

pub struct Record {
    pub id: RecordId,
    pub data: Option<Entry>,
    pub modified: DateTime<Local>,
}

impl Record {
    pub fn new(id: RecordId, data: Option<Entry>) -> Self {
        Self {
            id,
            data,
            modified: Local::now(),
        }
    }
}

/// Various failure modes for records.
#[derive(Debug)]
pub enum RecordError {
    InvalidRecordIdFormat(String),
    InvalidSource(RecordId),
    InvalidSubId(RecordId),
    DatabaseFailure(rusqlite::Error),
    Incomplete,
}

impl From<rusqlite::Error> for RecordError {
    fn from(err: rusqlite::Error) -> Self {
        RecordError::DatabaseFailure(err)
    }
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordError::InvalidRecordIdFormat(input) => {
                write!(
                    f,
                    "'{}' is not in the format of '<source>:<sub_id>'.",
                    input
                )
            }
            RecordError::InvalidSource(record_id) => {
                write!(f, "'{}' is not a valid source.", record_id.source())
            }
            RecordError::InvalidSubId(record_id) => write!(
                f,
                "'{}' is not a valid sub-id for the source '{}'.",
                record_id.sub_id(),
                record_id.source()
            ),
            RecordError::Incomplete => write!(f, "Incomplete record"),
            RecordError::DatabaseFailure(error) => write!(f, "Database failure: {}", error),
        }
    }
}

// we probably want something like this
// pub enum CitationKey {
//     Alias(String),
//     RecordId(String, usize),
// }

/// A source (`source`) with corresponding identity (`sub_id`), such as 'arxiv:0123.4567'
#[derive(Debug, Clone, Hash, PartialEq, Eq, DeserializeFromStr, SerializeDisplay)]
pub struct RecordId {
    full_id: String,
    source_length: usize,
}

impl RecordId {
    /// Get the source part of the record id.
    pub fn source(&self) -> &str {
        &self.full_id[0..self.source_length]
    }

    /// Get the part of the record id after the source.
    pub fn sub_id(&self) -> &str {
        &self.full_id[self.source_length + 1..]
    }

    /// Get the full record id.
    pub fn full_id(&self) -> &str {
        &self.full_id
    }
}

impl FromStr for RecordId {
    type Err = RecordError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let first_colon_position = input.find(':');
        match first_colon_position {
            Some(p) => {
                if p == 0 || p == input.len() - 1 {
                    return Err(RecordError::InvalidRecordIdFormat(input.to_string()));
                }
                // TODO: trim whitespace
                Ok(RecordId {
                    full_id: String::from(input),
                    source_length: p,
                })
            }
            None => Err(RecordError::InvalidRecordIdFormat(input.to_string())),
        }
    }
}

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.full_id)
    }
}
