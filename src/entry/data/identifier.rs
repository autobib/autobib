use serde_bibtex::token::{is_balanced, is_entry_key};

use super::{EntryTypeHeader, KeyHeader, ValueHeader};
use crate::error::RecordDataError;

/// A validated entry key (e.g. "key" in `@book{key, ..}`) which satisfies the following
/// requirements:
///
/// 1. has length at least `1`
/// 2. composed only of ASCII printable characters except `{}(),=\\#%\"`, or non-ASCII UTF-8.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntryKey<S = String>(pub(in crate::entry) S);

impl<S: AsRef<str>> EntryKey<S> {
    #[inline]
    pub fn try_new(s: S) -> Result<Self, RecordDataError> {
        let entry_key = s.as_ref();

        if !is_entry_key(entry_key) {
            return Err(RecordDataError::ContainsInvalidChar);
        }

        Ok(Self(s))
    }
}

impl EntryKey {
    /// A placeholder value used for displaying keys which are not valid bibtex.
    pub fn placeholder() -> Self {
        Self(":not_valid_bibtex".to_owned())
    }

    /// Substitute a character with a different entry key.
    #[inline]
    pub fn substitute<S: AsRef<str>>(&self, from: char, to: &EntryKey<S>) -> Option<Self> {
        self.0
            .find(from)
            .map(|_| Self(self.0.replace(from, to.as_ref())))
    }
}

/// A validated entry type (e.g. "article" in `@article{...}`) which satisfies the following
/// requirements:
///
/// 1. has length at least `1` and at most [`u8::MAX`].
/// 2. composed only of ASCII printable characters with `{}(),= \t\n\\#%\"` and
///    `A..=Z` removed.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntryType<S = String>(pub(in crate::entry) S);

impl<S: AsRef<str>> EntryType<S> {
    #[inline]
    pub fn try_new(s: S) -> Result<Self, RecordDataError> {
        let entry_type = s.as_ref();

        // Condition 1
        if entry_type.is_empty() || entry_type.len() > EntryTypeHeader::MAX as usize {
            return Err(RecordDataError::EntryTypeInvalidLength(entry_type.len()));
        }

        // Condition 2
        validate_ascii_identifier(entry_type.as_bytes())?;

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

    pub fn in_collection() -> Self {
        Self("incollection".to_owned())
    }

    pub fn article() -> Self {
        Self("article".to_owned())
    }
}

/// A validated field key (e.g. `author` in `...author = {...}`) which satisfies the following
/// requirements:
///
/// 1. has length at least `1` and at most [`KeyHeader::MAX`].
/// 2. composed only of ASCII printable characters with `{}(),= \t\n\\#%\"` and
///    `A..=Z` removed.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct FieldKey<S = String>(pub(in crate::entry) S);

impl<S: AsRef<str>> FieldKey<S> {
    #[inline]
    pub fn try_new(s: S) -> Result<Self, RecordDataError> {
        let key = s.as_ref();

        // Condition 1
        if key.is_empty() || key.len() > KeyHeader::MAX as usize {
            return Err(RecordDataError::KeyInvalidLength(key.len()));
        }

        // Condition 2
        validate_ascii_identifier(key.as_bytes())?;

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
                let mut inner = s.trim().to_owned();
                inner.make_ascii_lowercase();
                Self::try_new(inner)
            }
        }
    };
}

identifier_impl!(EntryType);
identifier_impl!(EntryKey);
identifier_impl!(FieldKey);
identifier_impl!(FieldValue);

/// Lookup table for bytes which could appear in an ASCII entry key or field key.
/// This is precisely the ASCII printable characters with `{}(),= \t\n\\#%\"` and
/// `A..=Z` removed.
static ASCII_IDENTIFIER_ALLOWED: [bool; 256] = {
    const PR: bool = false; // disallowed printable bytes
    const CT: bool = false; // non-printable ascii
    const NA: bool = false; // not ascii
    const UC: bool = false; // uppercase alpha
    const __: bool = true; // permitted bytes
    [
        //   1   2   3   4   5   6   7   8   9   A   B   C   D   E   F
        CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, // 0
        CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, CT, // 1
        CT, __, PR, PR, __, PR, __, __, PR, PR, __, __, PR, __, __, __, // 2
        __, __, __, __, __, __, __, __, __, __, __, __, __, PR, __, __, // 3
        __, UC, UC, UC, UC, UC, UC, UC, UC, UC, UC, UC, UC, UC, UC, UC, // 4
        UC, UC, UC, UC, UC, UC, UC, UC, UC, UC, UC, __, PR, __, __, __, // 5
        __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, // 6
        __, __, __, __, __, __, __, __, __, __, __, PR, __, PR, __, CT, // 7
        NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, // 8
        NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, // 9
        NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, // A
        NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, // B
        NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, // C
        NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, // D
        NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, // E
        NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, // F
    ]
};

#[inline]
pub fn validate_ascii_identifier(s: &[u8]) -> Result<&str, RecordDataError> {
    match s.iter().find(|&b| !ASCII_IDENTIFIER_ALLOWED[*b as usize]) {
        Some(_) => Err(RecordDataError::ContainsInvalidChar),
        // SAFETY: the only bytes permitted by ASCII_IDENTIFIER_ALLOWED are valid ASCII
        None => Ok(unsafe { std::str::from_utf8_unchecked(s) }),
    }
}
