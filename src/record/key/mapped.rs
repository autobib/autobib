use std::fmt;

use super::{Alias, AliasOrRemoteId, Identifier, RecordError, RecordId, RemoteId};

/// A wrapper struct for a [`RemoteId`] which has been transformed from an original key, for
/// instance through a sub_id normalization or an alias transform.
///
/// This struct has a special [`Display`](fmt::Display) implementation which shows both the key and
/// the original value if the original value exists.
#[derive(Debug)]
pub struct MappedKey<T = String> {
    /// The underlying key.
    pub mapped: RemoteId,
    /// The original value of the key, if normalization was applied.
    pub original: Option<T>,
}

impl<T> MappedKey<T> {
    /// Initialize for a key which was unchanged.
    pub fn unchanged(key: RemoteId) -> Self {
        Self {
            mapped: key,
            original: None,
        }
    }

    /// Initialize for a key which was mapped from some original value.
    pub fn mapped(key: RemoteId, original: T) -> Self {
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
    /// Construct a [`RemoteId`] from the provider and sub_id components.
    #[inline]
    pub fn mapped_from_parts(provider: &str, sub_id: &str) -> Result<Self, RecordError> {
        let mut full_id = String::with_capacity(provider.len() + sub_id.len() + 1);
        full_id.push_str(provider);
        full_id.push(':');
        full_id.push_str(sub_id);

        RecordId {
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

impl<T> From<MappedKey<T>> for RemoteId {
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

impl<T> Identifier for MappedKey<T> {
    fn name(&self) -> &str {
        self.mapped.name()
    }
}

/// Either an [`Alias`] or a [`RemoteId`].
#[derive(Debug)]
pub enum MappedAliasOrRemoteId {
    /// An [`Alias`], and a possible value that it was mapped to.
    Alias(Alias),
    /// A [`RemoteId`], which may have been mapped from the original `provider:sub_id`.
    RemoteId(MappedKey),
}

impl From<AliasOrRemoteId> for MappedAliasOrRemoteId {
    /// Convert the mapped alias variant into a mapped key, preserving the other values.
    fn from(value: AliasOrRemoteId) -> Self {
        match value {
            AliasOrRemoteId::Alias(alias, None) => Self::Alias(alias),
            AliasOrRemoteId::Alias(alias, Some(remote_id)) => {
                Self::RemoteId(MappedKey::mapped(remote_id, alias.into()))
            }
            AliasOrRemoteId::RemoteId(mapped_key) => Self::RemoteId(mapped_key),
        }
    }
}
