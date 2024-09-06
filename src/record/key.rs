use std::{fmt, str::FromStr};

use either::Either;
use serde::{Deserialize, Serialize};

use crate::{
    error::{RecordError, RecordErrorKind},
    provider::lookup_validator,
    CitationKey,
};

/// An unvalidated wrapper for user input representing either a `provider:sub_id` or an `alias`.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
pub struct RecordId {
    full_id: String,
    provider_len: Option<usize>,
}

impl RecordId {
    /// Convert a [`RecordId`] into either an [`Alias`] or a [`RemoteId`].
    ///
    /// The [`Alias`] conversion is infallible (validation only requires checking that the
    /// colon is not present) whereas the [`RemoteId`] conversion can fail if `provider` is
    /// invalid or if `sub_id` is invalid given the provider.
    pub fn resolve(self) -> Result<Either<Alias, RemoteId>, RecordError> {
        match self.provider_len {
            Some(_) => self.try_into().map(Either::Right),
            None => self.try_into().map(Either::Left),
        }
    }
}

impl CitationKey for RecordId {
    fn name(&self) -> &str {
        &self.full_id
    }
}

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.name().fmt(f)
    }
}

impl From<RecordId> for String {
    fn from(record_id: RecordId) -> Self {
        record_id.full_id
    }
}

/// Convert an `&str` to a [`RecordId`]. The input is whitespace-trimmed. Otherwise, this
/// implementation is very cheap and does no validation.
impl From<&str> for RecordId {
    fn from(s: &str) -> Self {
        let full_id: String = s.trim().into();
        let provider_len = full_id.find(':');
        Self {
            full_id,
            provider_len,
        }
    }
}

/// A validated `alias`.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct Alias(String);

impl From<Alias> for String {
    fn from(alias: Alias) -> Self {
        alias.0
    }
}

impl CitationKey for Alias {
    fn name(&self) -> &str {
        &self.0
    }
}

impl FromStr for Alias {
    type Err = RecordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RecordId::from(s).try_into()
    }
}

impl fmt::Display for Alias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl TryFrom<RecordId> for Alias {
    type Error = RecordError;

    fn try_from(record_id: RecordId) -> Result<Self, Self::Error> {
        if let RecordId {
            full_id: s,
            provider_len: None,
        } = record_id
        {
            if !s.is_empty() {
                Ok(Self(s))
            } else {
                Err(RecordError {
                    input: s,
                    kind: RecordErrorKind::EmptyAlias,
                })
            }
        } else {
            Err(RecordError {
                input: record_id.full_id,
                kind: RecordErrorKind::RecordIdIsNotAlias,
            })
        }
    }
}

/// A validated `provider:sub_id`.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone)]
pub struct RemoteId {
    full_id: String,
    provider_len: usize,
}

impl RemoteId {
    /// Get the `provider` part of the remote id.
    pub fn provider(&self) -> &str {
        &self.full_id[..self.provider_len]
    }

    /// Get the `sub_id` part of the remote id, after the separator.
    pub fn sub_id(&self) -> &str {
        &self.full_id[self.provider_len + 1..]
    }

    /// Construct a [`RemoteId`] from the provider and sub_id components.
    pub fn from_parts(provider: &str, sub_id: &str) -> Self {
        let mut full_id = provider.to_owned();
        full_id.push(':');
        full_id.push_str(sub_id);
        Self {
            full_id,
            provider_len: provider.len(),
        }
    }

    /// Create a new `local` [`RecordId`].
    pub fn local(sub_id: &str) -> Self {
        Self::from_parts("local", sub_id)
    }

    pub(crate) fn new_unchecked(full_id: String) -> Self {
        let provider_len = full_id.find(':').unwrap();
        Self {
            full_id,
            provider_len,
        }
    }
}

impl CitationKey for RemoteId {
    fn name(&self) -> &str {
        &self.full_id
    }
}

impl fmt::Display for RemoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.name().fmt(f)
    }
}

impl From<RemoteId> for String {
    fn from(remote_id: RemoteId) -> Self {
        remote_id.full_id
    }
}

impl TryFrom<RecordId> for RemoteId {
    type Error = RecordError;

    fn try_from(record_id: RecordId) -> Result<Self, Self::Error> {
        match record_id.provider_len {
            Some(provider_len) => {
                let remote_id = Self {
                    full_id: record_id.full_id,
                    provider_len,
                };

                if remote_id.sub_id().is_empty() {
                    Err(RecordError {
                        input: remote_id.name().into(),
                        kind: RecordErrorKind::EmptyProvider,
                    })
                } else if remote_id.sub_id().is_empty() {
                    Err(RecordError {
                        input: remote_id.name().into(),
                        kind: RecordErrorKind::EmptySubId,
                    })
                } else {
                    // perform cheap validation based on the provider
                    match lookup_validator(remote_id.provider()) {
                        Some(validator) if validator(remote_id.sub_id()) => Ok(remote_id),
                        Some(_) => Err(RecordError {
                            input: remote_id.full_id,
                            kind: RecordErrorKind::InvalidSubId,
                        }),
                        None => Err(RecordError {
                            input: remote_id.full_id,
                            kind: RecordErrorKind::InvalidProvider,
                        }),
                    }
                }
            }
            None => Err(RecordError {
                input: record_id.name().into(),
                kind: RecordErrorKind::RecordIdIsNotAlias,
            }),
        }
    }
}

impl FromStr for RemoteId {
    type Err = RecordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RecordId::from(s).try_into()
    }
}
