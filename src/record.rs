use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Local};
use serde_with::{DeserializeFromStr, SerializeDisplay};

use crate::entry::{Entry, KeyedEntry};

pub struct Record {
    pub key: CitationKey,
    pub data: Entry,
    pub modified: DateTime<Local>,
}

impl From<Record> for KeyedEntry {
    fn from(record: Record) -> KeyedEntry {
        KeyedEntry {
            key: record.key,
            contents: record.data,
        }
    }
}

impl Record {
    pub fn new(key: CitationKey, data: Entry) -> Self {
        Self {
            key,
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
            RecordError::DatabaseFailure(error) => write!(f, "Database failure: {}", error),
            RecordError::NetworkFailure(error) => write!(f, "Network failure: {}", error),
            RecordError::Incomplete => write!(f, "Incomplete record"),
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, DeserializeFromStr, SerializeDisplay)]
pub enum CitationKey {
    RecordId(RecordId),
    Alias(String),
}

impl CitationKey {
    pub fn as_str(&self) -> &str {
        match self {
            Self::RecordId(record_id) => record_id.sub_id(),
            Self::Alias(s) => s.as_str(),
        }
    }
}

pub enum CitationKeyErrorKind {
    EmptySource,
    EmptySubId,
    EmptyAlias,
}

pub struct CitationKeyError {
    input: String,
    kind: CitationKeyErrorKind,
}

impl CitationKeyError {
    pub fn new(input: String, kind: CitationKeyErrorKind) -> Self {
        Self { input, kind }
    }
}

impl fmt::Display for CitationKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid citation key '{}': ", self.input)?;
        match self.kind {
            CitationKeyErrorKind::EmptySource => {
                f.write_str("record identifier 'source' must be non-empty")
            }
            CitationKeyErrorKind::EmptySubId => {
                f.write_str("record identifier 'sub_id' must be non-empty")
            }
            CitationKeyErrorKind::EmptyAlias => f.write_str("alias must be non-empty"),
        }
    }
}

impl FromStr for CitationKey {
    type Err = CitationKeyError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.find(':') {
            Some(source_length) => {
                if source_length == 0 {
                    return Err(CitationKeyError::new(
                        input.to_string(),
                        CitationKeyErrorKind::EmptySource,
                    ));
                } else if source_length == input.len() - 1 {
                    return Err(CitationKeyError::new(
                        input.to_string(),
                        CitationKeyErrorKind::EmptySubId,
                    ));
                }
                let trimmed = input.trim();
                Ok(CitationKey::RecordId(RecordId {
                    full_id: String::from(trimmed),
                    source_length,
                }))
            }
            None => {
                if input.len() == 0 {
                    return Err(CitationKeyError::new(
                        input.to_string(),
                        CitationKeyErrorKind::EmptyAlias,
                    ));
                } else {
                    Ok(CitationKey::Alias(input.to_string()))
                }
            }
        }
    }
}

impl fmt::Display for CitationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecordId(record_id) => record_id.fmt(f),
            Self::Alias(s) => f.write_str(s),
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

    /// Construct a RecordId from the source and sub_id components.
    pub fn from_parts(source: &str, sub_id: &str) -> Self {
        let mut new = source.to_string();
        new.push_str(":");
        new.push_str(sub_id);
        Self {
            full_id: new,
            source_length: source.len(),
        }
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
