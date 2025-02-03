use serde_bibtex::token::is_balanced;

use super::{EntryTypeHeader, KeyHeader, ValueHeader};
use crate::error::RecordDataError;

/// A validated entry type (e.g. "article" in `@article{...}`) which satisfies the following
/// requirements:
///
/// 1. has length at least `1` and at most [`u8::MAX`].
/// 2. composed only of ASCII lowercase letters (from [`char::is_ascii_lowercase`]).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntryType<S = String>(pub(in crate::entry) S);

impl<S: AsRef<str>> EntryType<S> {
    #[inline]
    pub fn try_new(s: S) -> Result<Self, RecordDataError> {
        let entry_type = s.as_ref();

        if entry_type.is_empty() || entry_type.len() > EntryTypeHeader::MAX as usize {
            return Err(RecordDataError::EntryTypeInvalidLength(entry_type.len()));
        }

        if entry_type.bytes().any(|ch| !ch.is_ascii_lowercase()) {
            return Err(RecordDataError::EntryTypeNotAsciiLowercase);
        }

        Ok(Self(s))
    }

    pub fn to_owned(&self) -> EntryType {
        EntryType(self.0.as_ref().to_owned())
    }
}

impl EntryType<String> {
    pub fn misc() -> Self {
        Self("misc".to_owned())
    }

    pub fn preprint() -> Self {
        Self("preprint".to_owned())
    }

    pub fn book() -> Self {
        Self("book".to_owned())
    }
}

/// A validated field key (e.g. `author` in `...author = {...}`) which satisfies the following
/// requirements:
///
/// 1. has length at least `1` and at most [`KeyHeader::MAX`].
/// 2. composed only of ASCII lowercase letters (from [`char::is_ascii_lowercase`]).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldKey<S = String>(pub(in crate::entry) S);

impl<S: AsRef<str>> FieldKey<S> {
    #[inline]
    pub fn try_new(s: S) -> Result<Self, RecordDataError> {
        let key = s.as_ref();

        // Condition K1
        if key.is_empty() || key.len() > KeyHeader::MAX as usize {
            return Err(RecordDataError::KeyInvalidLength(key.len()));
        }

        // Condition K2
        if key.bytes().any(|b| !b.is_ascii_lowercase()) {
            return Err(RecordDataError::KeyNotAsciiLowercase);
        }

        Ok(Self(s))
    }

    pub fn to_owned(&self) -> FieldKey {
        FieldKey(self.0.as_ref().to_owned())
    }
}

// the field key requirements are stricted than the field value requirements
impl<S> From<FieldKey<S>> for FieldValue<S> {
    fn from(value: FieldKey<S>) -> Self {
        Self(value.0)
    }
}

/// A validated field value (e.g. `John Doe` in `...author = {John Doe}`) which satisfies the
/// following requirements:
///
/// 1. has length at most [`ValueHeader::MAX`].
/// 2. satisfies the balanced `{}` rule (from [`serde_bibtex::token::is_balanced`]).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldValue<S = String>(pub(in crate::entry) S);

impl<S: AsRef<str>> FieldValue<S> {
    #[inline]
    pub fn try_new(s: S) -> Result<Self, RecordDataError> {
        let value = s.as_ref();

        // Condition V1
        if value.len() > ValueHeader::MAX as usize {
            return Err(RecordDataError::ValueInvalidLength(value.len()));
        }

        // Condition V2
        if !is_balanced(value.as_bytes()) {
            return Err(RecordDataError::ValueNotBalanced);
        }

        Ok(Self(s))
    }

    pub fn to_owned(&self) -> FieldValue {
        FieldValue(self.0.as_ref().to_owned())
    }
}

macro_rules! identifier_impl {
    ($e:ident) => {
        impl<S: AsRef<str>> AsRef<str> for $e<S> {
            fn as_ref(&self) -> &str {
                self.0.as_ref()
            }
        }

        impl<S: ::std::fmt::Display> ::std::fmt::Display for $e<S> {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                self.0.fmt(f)
            }
        }

        // Borrow implementation for convenience of using `get.
        impl<S: AsRef<str>> ::std::borrow::Borrow<str> for $e<S> {
            fn borrow(&self) -> &str {
                self.0.as_ref()
            }
        }

        // Borrow implementation for convenience of using `get.
        impl ::std::borrow::Borrow<String> for $e<String> {
            fn borrow(&self) -> &String {
                &self.0
            }
        }

        impl<S: AsRef<str>> PartialEq<str> for $e<S> {
            fn eq(&self, other: &str) -> bool {
                self.0.as_ref().eq(other)
            }
        }

        impl ::std::str::FromStr for $e {
            type Err = RecordDataError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::try_new(s.trim().to_owned())
            }
        }
    };
}

identifier_impl!(EntryType);
identifier_impl!(FieldKey);
identifier_impl!(FieldValue);
