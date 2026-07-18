use std::fmt;

use super::{Alias, AliasOrId, AsKey, Identifier, Key, RecordError};

/// A wrapper struct for an [`Identifier`] which has been transformed from an original key, for
/// instance through a sub_id normalization or an alias transform.
///
/// This struct has a special [`Display`](fmt::Display) implementation which shows both the key and
/// the original value if the original value exists.
#[derive(Debug)]
pub struct MappedKey<T = String> {
    /// The underlying key.
    pub mapped: Identifier,
    /// The original value of the key, if normalization was applied.
    pub original: Option<T>,
}

impl<T> MappedKey<T> {
    /// Initialize for a key which was unchanged.
    pub fn unchanged(key: Identifier) -> Self {
        Self {
            mapped: key,
            original: None,
        }
    }

    /// Initialize for a key which was mapped from some original value.
    pub fn mapped(key: Identifier, original: T) -> Self {
        Self {
            mapped: key,
            original: Some(original),
        }
    }

    /// Returns whether or not this variant is mapped.
    pub fn is_mapped(&self) -> bool {
        self.original.is_some()
    }
}

impl MappedKey {
    /// Construct an [`Identifier`] from the provider and sub_id components.
    #[inline]
    pub fn mapped_from_parts(provider: &str, sub_id: &str) -> Result<Self, RecordError> {
        let mut full_id = String::with_capacity(provider.len() + sub_id.len() + 1);
        full_id.push_str(provider);
        full_id.push(':');
        full_id.push_str(sub_id);

        Key {
            full_id,
            provider_len: Some(provider.len()),
        }
        .resolve(&())
        .and_then(TryFrom::try_from)
    }
}

impl<T: Into<Self>> From<MappedKey<T>> for String {
    fn from(value: MappedKey<T>) -> Self {
        match value.original {
            Some(original) => original.into(),
            _ => value.mapped.into(),
        }
    }
}

impl<T> From<MappedKey<T>> for Identifier {
    fn from(value: MappedKey<T>) -> Self {
        value.mapped
    }
}

impl<T: fmt::Display> fmt::Display for MappedKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}'", self.mapped)?;
        if let Some(s) = &self.original {
            write!(f, " (converted from '{s}')")?;
        }
        Ok(())
    }
}

impl<T> AsKey for MappedKey<T> {
    fn as_key(&self) -> &str {
        self.mapped.as_key()
    }
}

/// Either an [`Alias`] or an [`Identifier`].
#[derive(Debug)]
pub enum MappedAliasOrId {
    /// An [`Alias`].
    Alias(Alias),
    /// A [`Identifier`], which may be a normalized form of the original `provider:sub_id` or may
    /// have been mapped from an alias using an alias transformation.
    Id(MappedKey),
}

impl From<AliasOrId> for MappedAliasOrId {
    /// Convert the mapped alias variant into a mapped key, preserving the other values.
    fn from(value: AliasOrId) -> Self {
        match value {
            AliasOrId::Alias(alias, None) => Self::Alias(alias),
            AliasOrId::Alias(alias, Some(id)) => Self::Id(MappedKey::mapped(id, alias.into())),
            AliasOrId::Id(mapped_key) => Self::Id(mapped_key),
        }
    }
}
