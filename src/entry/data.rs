//! # Abstraction over BibTeX data.
//! This module implements the mutable [`MutableEntryData`] and immutable [`RawEntryData`] types, which
//! represent the data inherent in a BibTeX entry.
//!
//! The data consists of the entry type (e.g. `article`) as well as the field keys and values (e.g. `title =
//! {Title}`).

#[cfg(test)]
mod tests;

mod identifier;
mod raw;

use std::{borrow::Borrow, cmp::PartialEq, collections::BTreeMap, iter::Iterator, sync::LazyLock};

use delegate::delegate;
use regex::Regex;

pub use identifier::{EntryKey, EntryType, FieldKey, FieldValue, validate_ascii_identifier};
pub(crate) use raw::{EntryTypeHeader, KeyHeader, ValueHeader};
pub use raw::{RawEntryData, RawRecordFieldsIter};

use crate::normalize::{Normalize, normalize_whitespace_str};

/// This trait represents types which encapsulate the data content of a single BibTeX entry.
///
/// In addition to the methods provided by [`EntryData`], this field also allows obtaining the
/// field value by borrowing against an alternative lifetime 'r
pub trait BorrowedEntryData<'r>: EntryData {
    /// Iterate over `(key, value)` pairs in order.
    fn fields_borrowed(&self) -> impl Iterator<Item = (&'r str, &'r str)>;

    /// Get the value of the field, borrowing from an underlying data buffer.
    fn get_field_borrowed(&self, field_name: &str) -> Option<&'r str> {
        for (key, val) in self.fields_borrowed() {
            if field_name > key {
                return None;
            }

            if field_name == key {
                return Some(val);
            }
        }
        None
    }
}

/// This trait represents types which encapsulate the data content of a single BibTeX entry.
///
/// # Safety
/// The caller is required to guarantee that:
///
/// 1. The entry type returned by `entry_type` satisfies the requirements detailed in
///    [`EntryType`].
/// 2. Each `(key, value)` pair returned by `fields` satisfies the requirements detailed in
///    [`FieldKey`] and [`FieldValue`] respectively.
/// 3. The `(key, value)` pairs are sorted by key and no key is repeated.
pub unsafe trait EntryData: PartialEq {
    /// Iterate over `(key, value)` pairs in order.
    fn fields(&self) -> impl Iterator<Item = (&str, &str)>;

    /// Get the `entry_type` as a string slice.
    fn entry_type(&self) -> &str;

    /// Get the exact size (in bytes) of the binary format representation of the [`EntryData`].
    ///
    /// The default implementation is performed by iterating over all fields.
    fn raw_len(&self) -> usize {
        1  // the size of the binary version header
            + (1 + self.entry_type().len()) // the entry type, plus the 1-byte header
            + self // the key value pairs, plus the 3-byte header
                .fields()
                .map(|(k, v)| 3 + k.len() + v.len())
                .sum::<usize>()
    }

    /// Get the value of the field.
    ///
    /// The default implementation iterates over all fields and returns the first match.
    fn get_field<'r>(&'r self, field_name: &str) -> Option<&'r str> {
        for (key, val) in self.fields() {
            if field_name < key {
                return None;
            }

            if field_name == key {
                return Some(val);
            }
        }
        None
    }

    /// Check if the field exists.
    ///
    /// The default implementation checks that `get_field` returns `Some(_)`.
    fn contains_field(&self, field_name: &str) -> bool {
        self.get_field(field_name).is_some()
    }
}

/// An in-memory [`EntryData`] implementation which supports addition and deletion of fields.
#[derive(Debug, PartialEq, Eq)]
pub struct MutableEntryData<S = String> {
    pub(super) entry_type: EntryType<S>,
    pub(super) fields: BTreeMap<FieldKey<S>, FieldValue<S>>,
}

impl<S: AsRef<str> + Ord + From<&'static str>> Default for MutableEntryData<S> {
    fn default() -> Self {
        Self::new(EntryType("misc".into()))
    }
}

/// The result of checking the current state of the `eprint` and `eprinttype` relative to a provided
/// key.
enum EPrintState<S> {
    /// No changes required.
    Ok,
    /// The `eprint` field corresponding to the provided key needs to be updated with the provided
    /// value.
    NeedsUpdate(S),
    /// The given key was not present in the record.
    MissingKey,
}

/// The outcome of resolving the conflict when using [`MutableEntryData::merge_with_callback`].
pub enum ConflictResolved {
    /// Keep the current data.
    Current,
    /// Replace with incoming data.
    Incoming,
    /// Use new data.
    New(FieldValue),
}

impl<'r> MutableEntryData<&'r str> {
    pub fn borrow_entry_data<D: EntryData>(data: &'r D) -> Self {
        let mut new = Self::new(EntryType(data.entry_type()));
        for (key, value) in data.fields() {
            new.fields.insert(FieldKey(key), FieldValue(value));
        }
        new
    }
}

impl MutableEntryData {
    pub fn from_entry_data<D: EntryData>(data: &D) -> Self {
        let mut new = Self::new(EntryType(data.entry_type().to_owned()));
        for (key, value) in data.fields() {
            new.fields
                .insert(FieldKey(key.to_owned()), FieldValue(value.to_owned()));
        }
        new
    }

    /// Check for the following configuration inside the data:
    /// ```bib
    ///   eprinttype = {key},
    ///   eprint = {val},
    ///   key = {val},
    /// ```
    /// If the key is missing, returns `EPrintState::Missing`; otherwise, check if the `eprinttype`
    /// and `eprint` keys require changing.
    fn is_eprint_normalized<Q: AsRef<str>>(&self, key: Q) -> EPrintState<&FieldValue> {
        let key_ref = key.as_ref();
        match self.get(key_ref) {
            Some(val) => {
                if self.get("eprinttype").is_some_and(|v| v.0 == key_ref)
                    && self.get("eprint").is_some_and(|v| v.0 == val.0)
                {
                    EPrintState::Ok
                } else {
                    EPrintState::NeedsUpdate(val)
                }
            }
            None => EPrintState::MissingKey,
        }
    }

    /// This method is very similar to `merge_or_overwrite`, but also updates the entry type and is
    /// slightly more optimized since it blindly overwrites existing entries, instead of checking
    /// that they are different.
    pub fn update_from<D: EntryData>(&mut self, data: &D) {
        self.entry_type.0.clear();
        self.entry_type.0.push_str(data.entry_type());

        for (key, value) in data.fields() {
            match self.fields.get_mut(key) {
                Some(existing) => {
                    existing.0.clear();
                    existing.0.push_str(value);
                }
                None => {
                    self.fields
                        .insert(FieldKey(key.to_owned()), FieldValue(value.to_owned()));
                }
            }
        }
    }

    /// Merge data from `other`, invoking a callback to resolve conflicts.
    ///
    /// The callback `resolve_conflict` takes three arguments in the following order:
    /// the key, the existing value in `self` corresponding to the key, and the new value.
    pub fn merge_with_callback<
        D: EntryData,
        C: FnMut(FieldKey<&str>, FieldValue<&str>, FieldValue<&str>) -> ConflictResolved,
    >(
        &mut self,
        other: &D,
        mut resolve_conflict: C,
    ) {
        for (key, value) in other.fields() {
            match self.fields.get_mut(key) {
                Some(current_value) if current_value != value => {
                    match resolve_conflict(
                        FieldKey(key),
                        FieldValue(&current_value.0),
                        FieldValue(value),
                    ) {
                        ConflictResolved::Current => continue,
                        ConflictResolved::Incoming => {
                            current_value.0.clear();
                            current_value.0.push_str(value);
                        }
                        ConflictResolved::New(new_value) => {
                            *current_value = new_value;
                        }
                    };
                }
                Some(_) => {}
                None => {
                    self.fields
                        .insert(FieldKey(key.to_owned()), FieldValue(value.to_owned()));
                }
            }
        }
    }

    /// Merge data from `other`, ignoring fields that already exist in `self`.
    #[inline]
    pub fn merge_or_skip<D: EntryData>(&mut self, other: &D) {
        self.merge_with_callback(other, |_, _, _| ConflictResolved::Current);
    }

    /// Merge data from `other`, overwriting fields that already exist in `self`.
    #[inline]
    pub fn merge_or_overwrite<D: EntryData>(&mut self, other: &D) {
        self.merge_with_callback(other, |_, _, _| ConflictResolved::Incoming);
    }

    pub fn try_new(e: String) -> Result<Self, crate::error::RecordDataError> {
        Ok(Self::new(EntryType::try_new(e)?))
    }

    pub fn check_and_insert(
        &mut self,
        k: String,
        v: String,
    ) -> Result<(), crate::error::RecordDataError> {
        self.insert(FieldKey::try_new(k)?, FieldValue::try_new(v)?);
        Ok(())
    }

    pub fn check_and_insert_if_non_null(
        &mut self,
        k: &str,
        v: Option<String>,
    ) -> Result<(), crate::error::RecordDataError> {
        if let Some(s) = v {
            self.insert(FieldKey::try_new(k.into())?, FieldValue::try_new(s)?);
        }
        Ok(())
    }
}

impl<S: AsRef<str> + Ord> MutableEntryData<S> {
    /// Initialize a new [`MutableEntryData`] instance.
    pub fn new(entry_type: EntryType<S>) -> Self {
        Self {
            entry_type,
            fields: BTreeMap::new(),
        }
    }

    pub fn get_str<Q>(&self, key: &Q) -> Option<&str>
    where
        FieldKey<S>: Borrow<Q> + Ord,
        Q: Ord + ?Sized,
    {
        self.get(key).map(AsRef::as_ref)
    }

    delegate! {
        to self.fields {
            pub fn len(&self) -> usize;
            pub fn is_empty(&self) -> bool;
            pub fn get<Q>(&self, key: &Q) -> Option<&FieldValue<S>>
                where FieldKey<S>: Borrow<Q>,
                      Q: Ord + ?Sized;
            pub fn keys(&self) -> std::collections::btree_map::Keys<'_, FieldKey<S>, FieldValue<S>>;
            pub fn values(&self) -> std::collections::btree_map::Values<'_, FieldKey<S>, FieldValue<S>>;
            pub fn insert(&mut self, key: FieldKey<S>, value: FieldValue<S>) -> Option<FieldValue<S>>;
            pub fn contains_key<Q>(&self, key: &Q) -> bool
            where
                FieldKey<S>: Borrow<Q>,
                Q: Ord + ?Sized;
            pub fn remove<Q>(&mut self, key: &Q) -> Option<FieldValue<S>>
            where
                FieldKey<S>: Borrow<Q>,
                Q: Ord + ?Sized;
        }
    }
}

unsafe impl<S: AsRef<str> + Ord> EntryData for MutableEntryData<S> {
    fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields
            .iter()
            .map(|(k, v)| (k.0.as_ref(), v.0.as_ref()))
    }

    fn entry_type(&self) -> &str {
        self.entry_type.as_ref()
    }

    fn get_field(&self, field_name: &str) -> Option<&str> {
        self.get(field_name).map(|v| v.0.as_ref())
    }
}

static TRAILING_JOURNAL_SERIES_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\([1-9][0-9]*\)$").unwrap());

impl Normalize for MutableEntryData {
    fn set_eprint<Q: AsRef<str>>(&mut self, keys: std::slice::Iter<'_, Q>) -> bool {
        for key in keys {
            match self.is_eprint_normalized(key) {
                EPrintState::Ok => {
                    return false;
                }
                EPrintState::NeedsUpdate(val) => {
                    self.insert(FieldKey("eprint".into()), FieldValue(val.0.clone()));
                    // SAFETY: 'eprinttype' satisfies the key requirements
                    // SAFETY: `key` is already a key in the database, and the requirements for
                    // keys are stricter than the requirements for values.
                    self.insert(
                        FieldKey("eprinttype".into()),
                        FieldValue(key.as_ref().to_owned()),
                    );
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
            if let Some(new_val) = normalize_whitespace_str(val.0.as_ref()) {
                updated = true;
                // SAFETY: the `normalize_whitespace` function always reduces the length of the
                // input, since it either deletes unused whitespace, or replaces whitespace
                // with ASCII space which has the smallest possible length (as bytes)
                *val = FieldValue(new_val);
            }
        }

        updated
    }

    fn strip_journal_series(&mut self) -> bool {
        if let Some(journal) = self.fields.get_mut("journal")
            && let Some(truncate_offset) = TRAILING_JOURNAL_SERIES_RE
                .find(journal.0.as_ref())
                .map(|m| m.start())
        {
            // SAFETY: the new value is a prefix of the previous value, and the regex
            // guarantees that it will not result in unbalanced {}
            journal.0.truncate(truncate_offset);
            return true;
        }
        false
    }
}
