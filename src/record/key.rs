mod mapped;

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{
    AsKey,
    config::AliasTransform,
    error::{
        AliasConversionError, AliasErrorKind, IdConversionError, IdErrorKind, RecordError,
        RecordErrorKind,
    },
    provider::{ValidationOutcomeExtended, validate_provider_sub_id},
};
pub use mapped::{MappedAliasOrId, MappedKey};

/// Resolve the provider and sub_id implicit inside the provided `full_id`.
///
/// # Safety
/// The caller guarantees that `&full_id[..provider_len]` and `&full_id[provider_len + 1..]`
/// are both valid sub-slices of `full_id`, and `full_id[provider_len] == ':'`.
#[inline]
fn resolve_provider_sub_id(
    full_id: String,
    provider_len: usize,
) -> Result<MappedKey, IdConversionError> {
    if provider_len + 1 == full_id.len() {
        Err(IdConversionError {
            input: full_id,
            kind: IdErrorKind::EmptySubId,
        })
    } else if provider_len == 0 {
        Err(IdConversionError {
            input: full_id,
            kind: IdErrorKind::EmptyProvider,
        })
    } else {
        let provider = &full_id[..provider_len];
        let sub_id = &full_id[provider_len + 1..];
        match validate_provider_sub_id(provider, sub_id) {
            ValidationOutcomeExtended::Valid => Ok(MappedKey::unchanged(
                Identifier::new_unchecked(full_id, provider_len),
            )),
            ValidationOutcomeExtended::Normalize(mut normalized) => {
                normalized.insert_str(0, &full_id[..provider_len + 1]);
                Ok(MappedKey::mapped(
                    Identifier::new_unchecked(normalized, provider_len),
                    full_id,
                ))
            }
            ValidationOutcomeExtended::InvalidSubId => Err(IdConversionError {
                input: full_id,
                kind: IdErrorKind::InvalidSubId,
            }),
            ValidationOutcomeExtended::InvalidProvider => Err(IdConversionError {
                input: full_id,
                kind: IdErrorKind::InvalidProvider,
            }),
        }
    }
}

/// An unvalidated wrapper for user input representing either a `provider:sub_id` or an `alias`.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize, PartialOrd, Ord)]
pub struct Key {
    full_id: String,
    provider_len: Option<usize>,
}

impl Key {
    /// Convert a [`Key`] into either an [`Alias`] or an [`Identifier`].
    ///
    /// The [`Alias`] conversion is infallible (validation only requires checking that the
    /// colon is not present) whereas the [`Identifier`] conversion can fail if `provider` is
    /// invalid or if `sub_id` is invalid given the provider.
    #[inline]
    pub fn resolve<A: AliasTransform>(self, alias_transform: &A) -> Result<AliasOrId, RecordError> {
        match self.provider_len {
            Some(provider_len) => resolve_provider_sub_id(self.full_id, provider_len)
                .map(AliasOrId::Id)
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
                        Ok(AliasOrId::Alias(alias, Some(resolved.mapped)))
                    } else {
                        Ok(AliasOrId::Alias(alias, None))
                    }
                }
            }
        }
    }
}

impl AsKey for Key {
    fn as_key(&self) -> &str {
        &self.full_id
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_key().fmt(f)
    }
}

impl From<Key> for String {
    fn from(key: Key) -> Self {
        key.full_id
    }
}

/// Convert an `&str` to a [`Key`]. The input is whitespace-trimmed. Otherwise, this
/// implementation is very cheap and does no validation.
impl From<&str> for Key {
    fn from(s: &str) -> Self {
        let full_id: String = s.trim().into();
        let provider_len = full_id.find(':');
        Self {
            full_id,
            provider_len,
        }
    }
}

/// Convert a `String` to a [`Key`]. The input is not whitespace-trimmed.
/// The implementation is very cheap and does no validation.
impl From<String> for Key {
    fn from(full_id: String) -> Self {
        let provider_len = full_id.find(':');
        Self {
            full_id,
            provider_len,
        }
    }
}

/// Either an [`Alias`] or an [`Identifier`].
#[derive(Debug)]
pub enum AliasOrId {
    /// An [`Alias`], and a possible value that it was mapped to.
    Alias(Alias, Option<Identifier>),
    /// A [`Identifier`], which may be a normalized form of the original `provider:sub_id`.
    Id(MappedKey),
}

impl From<AliasOrId> for String {
    fn from(value: AliasOrId) -> Self {
        match value {
            AliasOrId::Alias(alias, _) => alias.into(),
            AliasOrId::Id(maybe_transformed) => maybe_transformed.mapped.into(),
        }
    }
}

impl TryFrom<AliasOrId> for MappedKey {
    type Error = RecordError;

    #[inline]
    fn try_from(value: AliasOrId) -> Result<Self, Self::Error> {
        match value {
            AliasOrId::Alias(alias, _) => Err(Self::Error {
                input: alias.into(),
                kind: RecordErrorKind::Identifier(IdErrorKind::IsAlias),
            }),
            AliasOrId::Id(maybe_normalized) => Ok(maybe_normalized),
        }
    }
}

impl TryFrom<AliasOrId> for Identifier {
    type Error = RecordError;

    #[inline]
    fn try_from(value: AliasOrId) -> Result<Self, Self::Error> {
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

impl AsKey for Alias {
    fn as_key(&self) -> &str {
        &self.0
    }
}

impl FromStr for Alias {
    type Err = AliasConversionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let input = s.trim().to_owned();

        if input.is_empty() {
            return Err(AliasConversionError {
                input,
                kind: AliasErrorKind::Empty,
            });
        }

        if input.find(':').is_some() {
            return Err(AliasConversionError {
                input,
                kind: AliasErrorKind::IsIdentifier,
            });
        }

        if input.chars().any(char::is_control) {
            return Err(AliasConversionError {
                input,
                kind: AliasErrorKind::ContainsControl,
            });
        }

        Ok(Self(input))
    }
}

impl fmt::Display for Alias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A validated legacy `alias`.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct LegacyAlias(String);

impl FromStr for LegacyAlias {
    type Err = AliasConversionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let input = s.trim().to_owned();

        if input.is_empty() {
            return Err(AliasConversionError {
                input,
                kind: AliasErrorKind::Empty,
            });
        }

        if input.find(':').is_some() {
            return Err(AliasConversionError {
                input,
                kind: AliasErrorKind::IsIdentifier,
            });
        }

        Ok(Self(input))
    }
}

impl fmt::Display for LegacyAlias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for LegacyAlias {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<Key> for Alias {
    type Error = AliasConversionError;

    fn try_from(key: Key) -> Result<Self, Self::Error> {
        if let Key {
            full_id: s,
            provider_len: None,
        } = key
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
                input: key.full_id,
                kind: AliasErrorKind::IsIdentifier,
            })
        }
    }
}

/// A validated `provider:sub_id`.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone)]
pub struct Identifier<S = String> {
    full_id: S,
    provider_len: usize,
}

impl<S: AsRef<str>> Identifier<S> {
    /// Construct a new [`Identifier`], assuming that the struct has been validated.
    #[inline]
    fn new_unchecked(full_id: S, provider_len: usize) -> Self {
        Self {
            full_id,
            provider_len,
        }
    }

    /// Construct a new [`Identifier`] from the given full_id.
    ///
    /// # Safety
    /// The caller is required to guarantee that:
    /// 1. The `full_id` is not an [`Alias`], i.e. it contains a ':' symbol;
    /// 2. The `full_id` has a non-empty `provider` part, i.e. it does not start with ':';
    /// 3. The `full_id` has a non-empty `sub_id` part, i.e. the first ':' is not at the end; and
    /// 4. [`validate_provider_sub_id`] is valid.
    #[inline]
    pub(crate) fn from_string_unchecked(full_id: S) -> Self {
        Self::from_key_unchecked(full_id).unwrap()
    }

    /// Construct a new [`Identifier`] from the given identifier which might be an alias.
    ///
    /// # Safety
    /// The caller is required to guarantee that either the identifier is an alias, or:
    /// 1. The `full_id` has a non-empty `provider` part, i.e. it does not start with ':';
    /// 2. The `full_id` has a non-empty `sub_id` part, i.e. the first ':' is not at the end; and
    /// 3. [`validate_provider_sub_id`] is valid.
    #[inline]
    pub(crate) fn from_key_unchecked(full_id: S) -> Option<Self> {
        full_id
            .as_ref()
            .find(':')
            .map(|provider_len| Self::new_unchecked(full_id, provider_len))
    }

    /// Get the `provider` part of the remote id.
    #[inline]
    pub fn provider(&self) -> &str {
        &self.full_id.as_ref()[..self.provider_len]
    }

    /// Check whether the `provider` part of the remote id is `local`.
    #[inline]
    pub fn is_local(&self) -> bool {
        self.provider() == "local"
    }

    /// Get the `sub_id` part of the remote id, after the separator.
    #[inline]
    pub fn sub_id(&self) -> &str {
        &self.full_id.as_ref()[self.provider_len + 1..]
    }
}

impl Identifier {
    /// Construct an [`Identifier`] from the provider and sub_id components.
    #[inline]
    pub fn from_parts(provider: &str, sub_id: &str) -> Result<Self, RecordError> {
        MappedKey::mapped_from_parts(provider, sub_id).map(Into::into)
    }

    /// Forget that this is an [`Identifier`] and convert back into a [`Key`].
    pub fn forget(self) -> Key {
        Key {
            full_id: self.full_id,
            provider_len: Some(self.provider_len),
        }
    }

    /// Create a new `local` [`Key`].
    pub fn local(alias: &Alias) -> Self {
        const LOCAL_PROVIDER: &str = "local";
        const PROVIDER_LEN: usize = LOCAL_PROVIDER.len();

        let mut full_id = String::with_capacity(PROVIDER_LEN + alias.0.len() + 1);
        full_id.push_str(LOCAL_PROVIDER);
        full_id.push(':');
        full_id.push_str(alias.0.as_str());
        Self::new_unchecked(full_id, PROVIDER_LEN)
    }

    pub fn as_deref(&self) -> Identifier<&str> {
        Identifier {
            full_id: &self.full_id,
            provider_len: self.provider_len,
        }
    }
}

impl Identifier<&str> {
    pub fn as_owned(&self) -> Identifier {
        Identifier {
            full_id: self.full_id.into(),
            provider_len: self.provider_len,
        }
    }
}

impl<S: AsRef<str>> AsKey for Identifier<S> {
    fn as_key(&self) -> &str {
        self.full_id.as_ref()
    }
}

impl<S: AsRef<str>> fmt::Display for Identifier<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_key().fmt(f)
    }
}

impl From<Identifier> for String {
    fn from(id: Identifier) -> Self {
        id.full_id
    }
}

impl FromStr for Identifier {
    type Err = RecordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Key::from(s).resolve(&()).and_then(TryFrom::try_from)
    }
}
