//! # Abstraction over BibTeX data.
//! This module implements the mutable [`RecordData`] and immutable [`RawRecordData`] types, which
//! represent the data inherent in a BibTeX entry.
//!
//! The data consists of the entry type (e.g. `article`) as well as the field keys and values (e.g. `title =
//! {Title}`).

#[cfg(test)]
mod tests;

mod raw;

use std::{borrow::Borrow, cmp::PartialEq, collections::BTreeMap, iter::Iterator, sync::LazyLock};

pub use raw::{binary_format_version, RawRecordData};
pub(crate) use raw::{EntryTypeHeader, KeyHeader, ValueHeader};

use delegate::delegate;
use regex::Regex;
use serde_bibtex::token::is_balanced;

use crate::{
    error::RecordDataError,
    normalize::{normalize_whitespace_str, Normalize},
};

/// This trait represents types which encapsulate the data content of a single BibTeX entry.
pub trait EntryData: PartialEq {
    /// Iterate over `(key, value)` pairs in order.
    fn fields(&self) -> impl Iterator<Item = (&str, &str)>;

    /// Get the `entry_type` as a string slice.
    fn entry_type(&self) -> &str;

    /// Get the value of the field.
    ///
    /// The default implementation iterates over all fields and returns the first match.
    fn get_field(&self, field_name: &str) -> Option<&str> {
        self.fields()
            .find_map(|(key, val)| if field_name == key { Some(val) } else { None })
    }
}

// Hack to avoid conflicting trait implementations
trait EntryDataExt: EntryData {}
impl EntryDataExt for RecordData {}
impl EntryDataExt for &RecordData {}

impl<D: EntryDataExt> From<D> for RawRecordData {
    /// Convert a [`RecordData`] into a [`RawRecordData`] for insertion into the database.
    fn from(record_data: D) -> Self {
        let mut data = vec![binary_format_version()];

        let entry_type = record_data.entry_type();
        let entry_type_len = EntryTypeHeader::try_from(entry_type.len()).unwrap();
        data.push(entry_type_len);
        data.extend(entry_type.as_bytes());

        for (key, value) in record_data.fields() {
            let key_len = KeyHeader::try_from(key.len()).unwrap();
            let value_len = ValueHeader::try_from(value.len()).unwrap().to_le_bytes();

            data.push(key_len);
            data.extend(value_len);
            data.extend(key.as_bytes());
            data.extend(value.as_bytes());
        }

        // SAFETY: the invariants are upheld based on the
        // `RecordData::insert` implementation.
        unsafe { Self::from_byte_repr_unchecked(data) }
    }
}

/// An in-memory [`EntryData`] implementation which supports addition and deletion of fields.
///
/// This type is mutable, in that it supports addition via
/// [`RecordData::check_and_insert`] and deletion via [`RecordData::remove`]. Insertion is
/// fallible, since the contents of this struct must satisfy the requirements of the binary data
/// format as detailed in the [`db`](`crate::db`) module.
///
/// There are no methods to return mutable references to the underlying data.
#[derive(Debug, PartialEq, Eq)]
pub struct RecordData {
    entry_type: String,
    fields: BTreeMap<String, String>,
}

impl Default for RecordData {
    fn default() -> Self {
        // SAFETY: cannot fail since "misc" satisfies the size requirements
        Self::try_new("misc".to_owned()).unwrap()
    }
}

impl<D: EntryData> From<&D> for RecordData {
    fn from(data: &D) -> Self {
        let mut new = Self::new_unchecked(data.entry_type().to_owned());
        for (key, value) in data.fields() {
            new.fields.insert(key.to_owned(), value.to_owned());
        }
        new
    }
}

/// The result of checking the current state of the `eprint` and `eprinttype` relative to a provided
/// key.
enum EPrintState {
    /// No changes required.
    Ok,
    /// The `eprint` field corresponding to the provided key needs to be updated with the provided
    /// value.
    NeedsUpdate(String),
    /// The given key was not present in the record.
    MissingKey,
}

impl RecordData {
    /// Initialize a new [`RecordData`] instance.
    ///
    /// This is fallible since `entry_type` must satisfy the following requirements.
    ///
    /// 1. `entry_type` must have length at least `1` and at most [`u8::MAX`].
    /// 2. `entry_type` must be composed only of ASCII lowercase letters (from [`char::is_ascii_lowercase`]).
    pub fn try_new(entry_type: String) -> Result<Self, RecordDataError> {
        if entry_type.is_empty() || entry_type.len() > EntryTypeHeader::MAX as usize {
            return Err(RecordDataError::EntryTypeInvalidLength(entry_type.len()));
        }

        if entry_type.chars().any(|ch| !ch.is_ascii_lowercase()) {
            return Err(RecordDataError::EntryTypeNotAsciiLowercase);
        }

        Ok(Self::new_unchecked(entry_type))
    }

    fn new_unchecked(entry_type: String) -> Self {
        Self {
            entry_type,
            fields: BTreeMap::new(),
        }
    }

    /// Check that the given value satisfies the following conditions:
    /// V1. `value` must have length at most [`ValueHeader::MAX`].
    /// V2. `value` must satisfy the balanced `{}` rule (from [`serde_bibtex::token::is_balanced`]).
    ///
    /// This method is also useful for validating types in calling code before insertion,
    /// particularly when inserting data provided interactively by the user.
    #[inline]
    pub fn check_value(value: &str) -> Result<(), RecordDataError> {
        // Condition V1
        if value.len() > ValueHeader::MAX as usize {
            return Err(RecordDataError::ValueInvalidLength(value.len()));
        }

        // Condition V2
        if !is_balanced(value.as_bytes()) {
            return Err(RecordDataError::ValueNotBalanced);
        }

        Ok(())
    }

    /// Check that the given key satisfies the following conditions:
    /// K1. `key` must have length at least `1` and at most [`KeyHeader::MAX`].
    /// K2. `key` must be composed only of ASCII lowercase letters (from [`char::is_ascii_lowercase`]).
    ///
    /// This method is also useful for validating types in calling code before insertion,
    /// particularly when inserting data provided interactively by the user.
    #[inline]
    pub fn check_key(key: &str) -> Result<(), RecordDataError> {
        // Condition K1
        if key.is_empty() || key.len() > KeyHeader::MAX as usize {
            return Err(RecordDataError::KeyInvalidLength(key.len()));
        }

        // Condition K2
        if key.chars().any(|ch| !ch.is_ascii_lowercase()) {
            return Err(RecordDataError::KeyNotAsciiLowercase);
        }

        Ok(())
    }

    /// Attempt to insert a new `(key, value)` pair.
    ///
    /// The `key` rules from [`check_key`](Self::check_value) and the `value` rules from
    /// [`check_value`](Self::check_value) must be satisfied by the inserted key and value
    /// respectively.
    pub fn check_and_insert(
        &mut self,
        key: String,
        value: String,
    ) -> Result<Option<String>, RecordDataError> {
        // Conditions K1 and K2
        Self::check_key(&key)?;

        // Conditions V1 and V2
        Self::check_value(&value)?;

        Ok(self.fields.insert(key, value))
    }

    /// Merge data from `other`, overwriting fields that already exist in `self`.
    pub fn merge_or_overwrite<D: EntryData>(&mut self, other: &D) -> Result<(), RecordDataError> {
        for (key, value) in other.fields() {
            self.check_and_insert(key.to_owned(), value.to_owned())?;
        }
        Ok(())
    }

    /// Merge data from `other`, ignoring fields that already exist in `self`.
    pub fn merge_or_skip<D: EntryData>(&mut self, other: &D) -> Result<(), RecordDataError> {
        for (key, value) in other.fields() {
            if !self.fields.contains_key(key) {
                self.check_and_insert(key.to_owned(), value.to_owned())?;
            }
        }
        Ok(())
    }

    /// Merge data from `other`, invoking a callback to resolve conflicts.
    ///
    /// The callback `resolve_conflict` takes three arguments in the following order:
    /// the key, the existing value in `self` corresponding to the key, and the new value.
    pub fn merge_with_callback<D: EntryData, C: FnMut(&str, &str, &str) -> String>(
        &mut self,
        other: &D,
        mut resolve_conflict: C,
    ) -> Result<(), RecordDataError> {
        for (key, value) in other.fields() {
            match self.fields.get_mut(key) {
                Some(current_value) if current_value != value => {
                    let new_value = resolve_conflict(key, current_value, value);
                    // SAFETY: since the key already corresponds to an entry in the database, we
                    // only need to check that the value satisfies conditions V1 and V2, and we can
                    // do an in-place memory replace to avoid the additional checks.
                    Self::check_value(&new_value)?;
                    // This is more efficient than using `mem::swap` since this compiles down to a
                    // single memcpy.
                    let _ = std::mem::replace(current_value, new_value);
                }
                Some(_) => {}
                None => {
                    self.check_and_insert(key.to_owned(), value.to_owned())?;
                }
            }
        }
        Ok(())
    }

    /// Check for the following configuration inside the data:
    /// ```bib
    ///   eprinttype = {key},
    ///   eprint = {val},
    ///   key = {val},
    /// ```
    /// If the key is missing, returns `EPrintState::Missing`; otherwise, check if the `eprinttype`
    /// and `eprint` keys require changing.
    fn is_eprint_normalized<Q: AsRef<str>>(&self, key: Q) -> EPrintState {
        match self.fields.get(key.as_ref()) {
            Some(val) => {
                if self
                    .fields
                    .get("eprinttype")
                    .is_some_and(|k| k == key.as_ref())
                    && self.fields.get("eprint").is_some_and(|v| v == val)
                {
                    EPrintState::Ok
                } else {
                    EPrintState::NeedsUpdate(val.to_owned())
                }
            }
            None => EPrintState::MissingKey,
        }
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&str>
    where
        String: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.fields.get(key).map(String::as_str)
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        String: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.fields.contains_key(key)
    }

    pub fn remove<Q>(&mut self, key: &Q) -> Option<String>
    where
        String: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.fields.remove(key)
    }

    delegate! {
        to self.fields {
            pub fn len(&self) -> usize;
            pub fn is_empty(&self) -> bool;
            pub fn keys(&self) -> std::collections::btree_map::Keys<'_, String, String>;
            pub fn values(&self) -> std::collections::btree_map::Values<'_, String, String>;
        }
    }
}

impl EntryData for RecordData {
    fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    fn entry_type(&self) -> &str {
        &self.entry_type
    }

    fn get_field(&self, field_name: &str) -> Option<&str> {
        self.get(field_name)
    }
}

impl EntryData for &RecordData {
    fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    fn entry_type(&self) -> &str {
        &self.entry_type
    }

    fn get_field(&self, field_name: &str) -> Option<&str> {
        self.get(field_name)
    }
}

static TRAILING_JOURNAL_SERIES_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\([1-9][0-9]*\)$").unwrap());

impl Normalize for RecordData {
    fn set_eprint<Q: AsRef<str>>(&mut self, keys: std::slice::Iter<'_, Q>) -> bool {
        for key in keys {
            match self.is_eprint_normalized(key) {
                EPrintState::Ok => {
                    return false;
                }
                EPrintState::NeedsUpdate(val) => {
                    // SAFETY: 'eprint' satisfies the key requirements
                    // SAFETY: 'val' is already a value in the database, and therefore satisfies
                    // the 'value' requirements.
                    self.fields.insert("eprint".to_owned(), val);
                    // SAFETY: 'eprinttype' satisfies the key requirements
                    // SAFETY: `key` is already a key in the database, and the requirements for
                    // keys are stricter than the requirements for values.
                    self.fields
                        .insert("eprinttype".to_owned(), key.as_ref().to_owned());
                    return true;
                }
                EPrintState::MissingKey => {}
            }
        }
        false
    }

    fn normalize_whitespace(&mut self) -> bool {
        let mut updated = false;

        for val in self.fields.values_mut() {
            if let Some(new_val) = normalize_whitespace_str(val) {
                updated = true;
                // SAFETY: the `normalize_whitespace` function always reduces the length of the
                // input, since it either deletes unused whitespace, or replaces whitespace
                // with ASCII space which has the smallest possible length (as bytes)
                *val = new_val;
            }
        }

        updated
    }

    fn strip_journal_series(&mut self) -> bool {
        if let Some(journal) = self.fields.get_mut("journal") {
            if let Some(truncate_offset) =
                TRAILING_JOURNAL_SERIES_RE.find(journal).map(|m| m.start())
            {
                // SAFETY: the new value is a prefix of the previous value, and the regex
                // guarantees that it will not result in unbalanced {}
                journal.truncate(truncate_offset);
                return true;
            }
        }
        false
    }
}
