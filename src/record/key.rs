use std::fmt;
use std::str::FromStr;

use crate::source::lookup_validator;

use serde_with::{DeserializeFromStr, SerializeDisplay};

#[derive(Debug, Clone, Hash, PartialEq, Eq, DeserializeFromStr, SerializeDisplay)]
pub enum CitationKey {
    RecordId(RecordId),
    Alias(String),
}

impl CitationKey {
    pub fn as_str(&self) -> &str {
        match self {
            Self::RecordId(record_id) => record_id.full_id(),
            Self::Alias(s) => s.as_str(),
        }
    }
}

pub enum CitationKeyErrorKind {
    EmptySource,
    EmptySubId,
    EmptyAlias,
    InvalidSource,
    InvalidSubId,
}

pub struct CitationKeyError {
    input: String,
    kind: CitationKeyErrorKind,
}

impl CitationKeyError {
    pub fn new(input: &str, kind: CitationKeyErrorKind) -> Self {
        Self {
            input: input.to_string(),
            kind,
        }
    }
}

impl fmt::Display for CitationKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid citation key '{}': ", self.input)?;
        match self.kind {
            CitationKeyErrorKind::EmptySource => f.write_str("'source' must be non-empty"),
            CitationKeyErrorKind::EmptySubId => f.write_str("'sub_id' must be non-empty"),
            CitationKeyErrorKind::EmptyAlias => f.write_str("alias must be non-empty"),
            CitationKeyErrorKind::InvalidSource => f.write_str("'source' is invalid"),
            CitationKeyErrorKind::InvalidSubId => {
                f.write_str("'sub_id' is invalid for the provided source")
            }
        }
    }
}

impl FromStr for CitationKey {
    type Err = CitationKeyError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        match input.find(':') {
            Some(source_length) => {
                if source_length == 0 {
                    return Err(CitationKeyError::new(
                        input,
                        CitationKeyErrorKind::EmptySource,
                    ));
                } else if source_length == input.len() - 1 {
                    return Err(CitationKeyError::new(
                        input,
                        CitationKeyErrorKind::EmptySubId,
                    ));
                }

                // check that the source and sub_id are valid
                let record_id = RecordId {
                    full_id: String::from(input),
                    source_length,
                };
                match lookup_validator(record_id.source()) {
                    Some(validator) if validator(record_id.sub_id()) => {
                        Ok(CitationKey::RecordId(record_id))
                    }
                    Some(_) => Err(CitationKeyError::new(
                        input,
                        CitationKeyErrorKind::InvalidSubId,
                    )),
                    None => Err(CitationKeyError::new(
                        input,
                        CitationKeyErrorKind::InvalidSource,
                    )),
                }
            }
            None => {
                if input.len() == 0 {
                    return Err(CitationKeyError::new(
                        input,
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
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct RecordId {
    full_id: String,
    source_length: usize,
}

impl RecordId {
    /// Get the source part of the record id.
    pub fn source(&self) -> &str {
        &self.full_id[..self.source_length]
    }

    /// Get the part of the record id after the source.
    pub fn sub_id(&self) -> &str {
        &self.full_id[self.source_length + 1..]
    }

    /// Get the full record id.
    pub fn full_id(&self) -> &str {
        &self.full_id
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
}

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.full_id)
    }
}
