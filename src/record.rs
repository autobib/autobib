use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Local};
use serde_with::{DeserializeFromStr, SerializeDisplay};

use crate::entry::{AnonymousEntry, Entry};

pub struct Record {
    pub id: RecordId,
    pub data: Option<AnonymousEntry>,
    pub modified: DateTime<Local>,
}

impl TryFrom<Record> for Entry {
    type Error = RecordError;

    fn try_from(record: Record) -> Result<Entry, RecordError> {
        match record.data {
            Some(contents) => Ok(Entry {
                key: record.id.into_string(),
                contents,
            }),
            None => Err(RecordError::NullRecord(record.id)),
        }
    }
}

impl Record {
    pub fn new(id: RecordId, data: Option<AnonymousEntry>) -> Self {
        Self {
            id,
            data,
            modified: Local::now(),
        }
    }
}

// TODO: subdivide this into smaller error groups
/// Various failure modes for records.
#[derive(Debug)]
pub enum RecordError {
    InvalidRecordIdFormat(String),
    InvalidSource(RecordId),
    InvalidSubId(RecordId),
    NullRecord(RecordId),
    NetworkFailure(reqwest::Error),
    DatabaseFailure(rusqlite::Error),
    Incomplete,
}

impl From<rusqlite::Error> for RecordError {
    fn from(err: rusqlite::Error) -> Self {
        RecordError::DatabaseFailure(err)
    }
}

impl From<reqwest::Error> for RecordError {
    fn from(err: reqwest::Error) -> Self {
        RecordError::NetworkFailure(err)
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
            RecordError::NullRecord(record_id) => write!(f, "'{}' is a null record", record_id),
            RecordError::DatabaseFailure(error) => write!(f, "Database failure: {}", error),
            RecordError::NetworkFailure(error) => write!(f, "Network failure: {}", error),
            RecordError::Incomplete => write!(f, "Incomplete record"),
        }
    }
}

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

    /// Return the underlying string
    pub fn into_string(self) -> String {
        self.full_id
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
