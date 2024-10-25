use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{
    error::{AliasConversionError, RecordError, RecordErrorKind},
    provider::{validate_provider_sub_id, ValidationOutcome},
    CitationKey,
};

/// An unvalidated wrapper for user input representing either a `provider:sub_id` or an `alias`.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize, PartialOrd, Ord)]
pub struct RecordId {
    full_id: String,
    provider_len: Option<usize>,
}

/// Either an [`Alias`] or a [`RemoteId]`.
pub enum AliasOrRemoteId {
    Alias(Alias),
    RemoteId(RemoteId),
}

impl RecordId {
    /// Convert a [`RecordId`] into either an [`Alias`] or a [`RemoteId`].
    ///
    /// The [`Alias`] conversion is infallible (validation only requires checking that the
    /// colon is not present) whereas the [`RemoteId`] conversion can fail if `provider` is
    /// invalid or if `sub_id` is invalid given the provider.
    pub fn resolve(self) -> Result<AliasOrRemoteId, RecordError> {
        match self.provider_len {
            Some(_) => self.try_into().map(AliasOrRemoteId::RemoteId),
            None => Ok(self.try_into().map(AliasOrRemoteId::Alias)?),
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
    type Err = AliasConversionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            Err(AliasConversionError::Empty(s.to_owned()))
        } else {
            match s.find(':') {
                Some(_) => Err(AliasConversionError::IsRemoteId(s.to_owned())),
                None => Ok(Self(s.to_owned())),
            }
        }
    }
}

impl fmt::Display for Alias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl TryFrom<RecordId> for Alias {
    type Error = AliasConversionError;

    fn try_from(record_id: RecordId) -> Result<Self, Self::Error> {
        if let RecordId {
            full_id: s,
            provider_len: None,
        } = record_id
        {
            if !s.is_empty() {
                Ok(Self(s))
            } else {
                Err(AliasConversionError::Empty(s))
            }
        } else {
            Err(AliasConversionError::IsRemoteId(record_id.full_id))
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
    /// Construct a new [`RemoteId`], assuming that the struct has been validated.
    #[inline]
    fn new_unchecked(full_id: String, provider_len: usize) -> Self {
        Self {
            full_id,
            provider_len,
        }
    }

    /// Construct a new [`RemoteId`] from the given full_id.
    ///
    /// # Safety
    /// The caller is required to guarantee that:
    /// 1. The `full_id` is not an [`Alias`], i.e. it contains a ':' symbol;
    /// 2. The `full_id` has a non-empty `provider` part, i.e. it does not start with ':';
    /// 3. The `full_id` has a non-empty `sub_id` part, i.e. the first ':' is not at the end; and
    /// 4. [`validate_provider_sub_id`] is valid.
    #[inline]
    pub(crate) unsafe fn from_string_unchecked(full_id: String) -> Self {
        let provider_len = full_id.find(':').unwrap();
        Self::new_unchecked(full_id, provider_len)
    }

    /// Construct a new [`RemoteId`], checking that the `provider` and `sub_id` components are
    /// valid.
    pub fn new(full_id: String, provider_len: usize) -> Result<Self, RecordError> {
        let remote_id = Self::new_unchecked(full_id, provider_len);
        if provider_len == 0 {
            Err(RecordError {
                input: remote_id.full_id,
                kind: RecordErrorKind::EmptyProvider,
            })
        } else if provider_len + 1 == remote_id.full_id.len() {
            Err(RecordError {
                input: remote_id.full_id,
                kind: RecordErrorKind::EmptySubId,
            })
        } else {
            match validate_provider_sub_id(remote_id.provider(), remote_id.sub_id()) {
                ValidationOutcome::Valid => Ok(remote_id),
                ValidationOutcome::InvalidSubId => Err(RecordError {
                    input: remote_id.into(),
                    kind: RecordErrorKind::InvalidSubId,
                }),
                ValidationOutcome::InvalidProvider => Err(RecordError {
                    input: remote_id.into(),
                    kind: RecordErrorKind::InvalidProvider,
                }),
            }
        }
    }

    /// Get the `provider` part of the remote id.
    #[inline]
    pub fn provider(&self) -> &str {
        &self.full_id[..self.provider_len]
    }

    /// Get the `sub_id` part of the remote id, after the separator.
    #[inline]
    pub fn sub_id(&self) -> &str {
        &self.full_id[self.provider_len + 1..]
    }

    /// Construct a [`RemoteId`] from the provider and sub_id components.
    pub fn from_parts(provider: &str, sub_id: &str) -> Result<Self, RecordError> {
        let mut full_id = String::with_capacity(provider.len() + sub_id.len() + 1);
        full_id.push_str(provider);
        full_id.push(':');
        full_id.push_str(sub_id);
        Self::new(full_id, provider.len())
    }

    /// Create a new `local` [`RecordId`].
    pub fn local(alias: &Alias) -> Self {
        const LOCAL_PROVIDER: &str = "local";
        const PROVIDER_LEN: usize = LOCAL_PROVIDER.len();

        let total_len = PROVIDER_LEN + 1 + alias.0.len();
        let mut full_id = String::with_capacity(total_len);
        full_id.push_str(LOCAL_PROVIDER);
        full_id.push(':');
        full_id.push_str(alias.0.as_str());
        Self::new_unchecked(full_id, PROVIDER_LEN)
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
            Some(provider_len) => RemoteId::new(record_id.full_id, provider_len),
            None => Err(RecordError {
                input: record_id.full_id,
                kind: RecordErrorKind::RecordIdIsNotRemoteId,
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
