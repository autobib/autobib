mod mapped;

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{
    config::AliasTransform,
    error::{
        AliasConversionError, AliasErrorKind, RecordError, RecordErrorKind,
        RemoteIdConversionError, RemoteIdErrorKind,
    },
    provider::{validate_provider_sub_id, ValidationOutcomeExtended},
    CitationKey,
};
pub use mapped::{MappedAliasOrRemoteId, MappedKey};

/// Resolve the provider and sub_id implicit inside the provided `full_id`.
///
/// # Safety
/// The caller guarantees that `&full_id[..provider_len]` and `&full_id[provider_len + 1..]`
/// are both valid sub-slices of `full_id`, and `full_id[provider_len] == ':'`.
#[inline]
fn resolve_provider_sub_id(
    full_id: String,
    provider_len: usize,
) -> Result<MappedKey, RemoteIdConversionError> {
    if provider_len + 1 == full_id.len() {
        Err(RemoteIdConversionError {
            input: full_id,
            kind: RemoteIdErrorKind::EmptySubId,
        })
    } else if provider_len == 0 {
        Err(RemoteIdConversionError {
            input: full_id,
            kind: RemoteIdErrorKind::EmptyProvider,
        })
    } else {
        let provider = &full_id[..provider_len];
        let sub_id = &full_id[provider_len + 1..];
        match validate_provider_sub_id(provider, sub_id) {
            ValidationOutcomeExtended::Valid => Ok(MappedKey::unchanged(RemoteId::new_unchecked(
                full_id,
                provider_len,
            ))),
            ValidationOutcomeExtended::Normalize(mut normalized) => {
                normalized.insert_str(0, &full_id[..provider_len + 1]);
                Ok(MappedKey::mapped(
                    RemoteId::new_unchecked(normalized, provider_len),
                    full_id,
                ))
            }
            ValidationOutcomeExtended::InvalidSubId => Err(RemoteIdConversionError {
                input: full_id,
                kind: RemoteIdErrorKind::InvalidSubId,
            }),
            ValidationOutcomeExtended::InvalidProvider => Err(RemoteIdConversionError {
                input: full_id,
                kind: RemoteIdErrorKind::InvalidProvider,
            }),
        }
    }
}

/// An unvalidated wrapper for user input representing either a `provider:sub_id` or an `alias`.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize, PartialOrd, Ord)]
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
    #[inline]
    pub fn resolve<A: AliasTransform>(
        self,
        alias_transform: &A,
    ) -> Result<AliasOrRemoteId, RecordError> {
        match self.provider_len {
            Some(provider_len) => resolve_provider_sub_id(self.full_id, provider_len)
                .map(AliasOrRemoteId::RemoteId)
                .map_err(Into::into),
            None => {
                if self.full_id.is_empty() {
                    Err(RecordError {
                        input: self.full_id,
                        kind: RecordErrorKind::Alias(AliasErrorKind::Empty),
                    })
                } else {
                    let alias = Alias(self.full_id);
                    if let Some((provider, sub_id)) = alias_transform.map_alias(&alias) {
                        let mut full_id = String::with_capacity(provider.len() + sub_id.len() + 1);
                        full_id.push_str(provider);
                        full_id.push(':');
                        full_id.push_str(sub_id);
                        let resolved = match resolve_provider_sub_id(full_id, provider.len()) {
                            Ok(resolved) => resolved,
                            Err(e) => {
                                // instead of calling `e.into()`, we preserve the original unmapped
                                // alias as the input
                                return Err(RecordError {
                                    input: alias.into(),
                                    kind: RecordErrorKind::InvalidMappedAlias(e.kind),
                                });
                            }
                        };
                        Ok(AliasOrRemoteId::Alias(alias, Some(resolved.mapped)))
                    } else {
                        Ok(AliasOrRemoteId::Alias(alias, None))
                    }
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

/// Either an [`Alias`] or a [`RemoteId`].
#[derive(Debug)]
pub enum AliasOrRemoteId {
    /// An [`Alias`], and a possible value that it was mapped to.
    Alias(Alias, Option<RemoteId>),
    /// A [`RemoteId`], which may have been mapped from the original `provider:sub_id`.
    RemoteId(MappedKey),
}

impl From<AliasOrRemoteId> for String {
    fn from(value: AliasOrRemoteId) -> Self {
        match value {
            AliasOrRemoteId::Alias(alias, _) => alias.into(),
            AliasOrRemoteId::RemoteId(maybe_transformed) => maybe_transformed.mapped.into(),
        }
    }
}

impl TryFrom<AliasOrRemoteId> for MappedKey {
    type Error = RecordError;

    #[inline]
    fn try_from(value: AliasOrRemoteId) -> Result<Self, Self::Error> {
        match value {
            AliasOrRemoteId::Alias(alias, _) => Err(Self::Error {
                input: alias.into(),
                kind: RecordErrorKind::RemoteId(RemoteIdErrorKind::IsAlias),
            }),
            AliasOrRemoteId::RemoteId(maybe_normalized) => Ok(maybe_normalized),
        }
    }
}

impl TryFrom<AliasOrRemoteId> for RemoteId {
    type Error = RecordError;

    #[inline]
    fn try_from(value: AliasOrRemoteId) -> Result<Self, Self::Error> {
        MappedKey::try_from(value).map(|k| k.mapped)
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
            Err(AliasConversionError {
                input: s.to_owned(),
                kind: AliasErrorKind::Empty,
            })
        } else {
            match trimmed.find(':') {
                Some(_) => Err(AliasConversionError {
                    input: s.to_owned(),
                    kind: AliasErrorKind::IsRemoteId,
                }),
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
                Err(AliasConversionError {
                    input: s,
                    kind: AliasErrorKind::Empty,
                })
            }
        } else {
            Err(AliasConversionError {
                input: record_id.full_id,
                kind: AliasErrorKind::IsRemoteId,
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
        Self::from_alias_or_remote_id_unchecked(full_id).unwrap()
    }

    /// Construct a new [`RemoteId`] from the given identifier which might be an alias.
    ///
    /// # Safety
    /// The caller is required to guarantee that either the identifier is an alias, or:
    /// 1. The `full_id` has a non-empty `provider` part, i.e. it does not start with ':';
    /// 2. The `full_id` has a non-empty `sub_id` part, i.e. the first ':' is not at the end; and
    /// 3. [`validate_provider_sub_id`] is valid.
    #[inline]
    pub(crate) fn from_alias_or_remote_id_unchecked(full_id: String) -> Option<Self> {
        full_id
            .find(':')
            .map(|provider_len| Self::new_unchecked(full_id, provider_len))
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
    #[inline]
    pub fn from_parts(provider: &str, sub_id: &str) -> Result<Self, RecordError> {
        MappedKey::mapped_from_parts(provider, sub_id).map(Into::into)
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
        RecordId::from(s).resolve(&()).and_then(TryFrom::try_from)
    }
}
