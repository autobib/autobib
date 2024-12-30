use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{
    error::{AliasConversionError, RecordError, RecordErrorKind},
    provider::{validate_provider_sub_id, ValidationOutcomeExtended},
    CitationKey,
};

/// A wrapper struct for a citation key (such as a [`RemoteId`] or an [`Alias`]) which has been
/// transformed from an original key, for instance through a sub_id normalization.
///
/// This struct has a special [`Display`](fmt::Display) implementation which shows both the key and
/// the original value if the original value exists.
#[derive(Debug)]
pub struct MappedKey<T> {
    /// The underlying key.
    pub key: T,
    /// The original value of the key, if normalization was applied.
    pub original: Option<String>,
}

impl<T> MappedKey<T> {
    /// Initialize for a key which was unchanged.
    pub fn unchanged(key: T) -> Self {
        Self {
            key,
            original: None,
        }
    }

    /// Initialize for a key which was mapped from some original value.
    pub fn mapped(key: T, original: String) -> Self {
        Self {
            key,
            original: Some(original),
        }
    }

    /// Returns whether or not this variant is mapped.
    pub fn is_mapped(&self) -> bool {
        self.original.is_some()
    }
}

impl<T: Into<String>> From<MappedKey<T>> for String {
    fn from(value: MappedKey<T>) -> Self {
        if let Some(original) = value.original {
            original
        } else {
            value.key.into()
        }
    }
}

impl<T: fmt::Display> fmt::Display for MappedKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}'", self.key)?;
        if let Some(s) = &self.original {
            write!(f, " (converted from '{s}')")?;
        }
        Ok(())
    }
}

impl<T: CitationKey> CitationKey for MappedKey<T> {
    fn name(&self) -> &str {
        self.key.name()
    }
}

/// An unvalidated wrapper for user input representing either a `provider:sub_id` or an `alias`.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize, PartialOrd, Ord)]
pub struct RecordId {
    full_id: String,
    provider_len: Option<usize>,
}

/// Either an [`Alias`] or a [`RemoteId`].
#[derive(Debug)]
pub enum AliasOrRemoteId {
    Alias(Alias),
    RemoteId(MappedKey<RemoteId>),
}

impl From<AliasOrRemoteId> for String {
    fn from(value: AliasOrRemoteId) -> Self {
        match value {
            AliasOrRemoteId::Alias(alias) => alias.into(),
            AliasOrRemoteId::RemoteId(maybe_transformed) => maybe_transformed.key.into(),
        }
    }
}

impl fmt::Display for AliasOrRemoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AliasOrRemoteId::Alias(alias) => alias.fmt(f),
            AliasOrRemoteId::RemoteId(maybe_transformed) => maybe_transformed.key.fmt(f),
        }
    }
}

impl TryFrom<AliasOrRemoteId> for RemoteId {
    type Error = RecordError;

    #[inline]
    fn try_from(value: AliasOrRemoteId) -> Result<Self, Self::Error> {
        match value {
            AliasOrRemoteId::Alias(alias) => Err(RecordError {
                input: alias.into(),
                kind: RecordErrorKind::RecordIdIsNotRemoteId,
            }),
            AliasOrRemoteId::RemoteId(maybe_normalized) => Ok(maybe_normalized.key),
        }
    }
}

impl TryFrom<AliasOrRemoteId> for Alias {
    type Error = RecordError;

    #[inline]
    fn try_from(value: AliasOrRemoteId) -> Result<Self, Self::Error> {
        match value {
            AliasOrRemoteId::Alias(alias) => Ok(alias),
            AliasOrRemoteId::RemoteId(maybe_normalized) => Err(RecordError {
                input: maybe_normalized.to_string(),
                kind: RecordErrorKind::RecordIdIsNotAlias,
            }),
        }
    }
}

impl RecordId {
    /// Convert a [`RecordId`] into either an [`Alias`] or a [`RemoteId`].
    ///
    /// The [`Alias`] conversion is infallible (validation only requires checking that the
    /// colon is not present) whereas the [`RemoteId`] conversion can fail if `provider` is
    /// invalid or if `sub_id` is invalid given the provider.
    #[inline]
    pub fn resolve(self) -> Result<AliasOrRemoteId, RecordError> {
        match self.provider_len {
            Some(provider_len) => {
                if provider_len == 0 {
                    Err(RecordError {
                        input: self.full_id,
                        kind: RecordErrorKind::EmptyProvider,
                    })
                } else if provider_len + 1 == self.full_id.len() {
                    Err(RecordError {
                        input: self.full_id,
                        kind: RecordErrorKind::EmptySubId,
                    })
                } else {
                    let provider = &self.full_id[..provider_len];
                    let sub_id = &self.full_id[provider_len + 1..];
                    match validate_provider_sub_id(provider, sub_id) {
                        ValidationOutcomeExtended::Valid => {
                            Ok(AliasOrRemoteId::RemoteId(MappedKey::unchanged(
                                RemoteId::new_unchecked(self.full_id, provider_len),
                            )))
                        }
                        ValidationOutcomeExtended::Normalize(mut normalized) => {
                            normalized.insert_str(0, &self.full_id[..provider_len + 1]);
                            Ok(AliasOrRemoteId::RemoteId(MappedKey::mapped(
                                RemoteId::new_unchecked(normalized, provider_len),
                                self.full_id,
                            )))
                        }
                        ValidationOutcomeExtended::InvalidSubId => Err(RecordError {
                            input: self.full_id,
                            kind: RecordErrorKind::InvalidSubId,
                        }),
                        ValidationOutcomeExtended::InvalidProvider => Err(RecordError {
                            input: self.full_id,
                            kind: RecordErrorKind::InvalidProvider,
                        }),
                    }
                }
            }
            None => {
                if self.full_id.is_empty() {
                    Err(RecordError {
                        input: self.full_id,
                        kind: RecordErrorKind::EmptyAlias,
                    })
                } else {
                    Ok(AliasOrRemoteId::Alias(Alias(self.full_id)))
                }
            }
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
        let trimmed = s.trim();
        if trimmed.is_empty() {
            Err(AliasConversionError::Empty(s.to_owned()))
        } else {
            match trimmed.find(':') {
                Some(_) => Err(AliasConversionError::IsRemoteId(s.to_owned())),
                None => Ok(Self(trimmed.to_owned())),
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

    /// Get the `provider` part of the remote id.
    #[inline]
    pub fn provider(&self) -> &str {
        &self.full_id[..self.provider_len]
    }

    /// Check whether the `provider` part of the remote id is `local`.
    #[inline]
    pub fn is_local(&self) -> bool {
        self.provider() == "local"
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

        RecordId {
            full_id,
            provider_len: Some(provider.len()),
        }
        .resolve()
        .and_then(TryFrom::try_from)
    }

    /// Create a new `local` [`RecordId`].
    pub fn local(alias: &Alias) -> Self {
        const LOCAL_PROVIDER: &str = "local";
        const PROVIDER_LEN: usize = LOCAL_PROVIDER.len();

        let mut full_id = String::with_capacity(PROVIDER_LEN + alias.0.len() + 1);
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

impl FromStr for RemoteId {
    type Err = RecordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RecordId::from(s).resolve().and_then(TryFrom::try_from)
    }
}
