//! # Abstraction over BibTeX data.
//! This module implements the mutable [`RecordData`] and immutable [`RawRecordData`] types, which
//! represent the data inherent in a BibTeX entry.
//!
//! The data consists of the entry type (e.g. `article`) as well as the field keys and values (e.g. `title =
//! {Title}`).
use std::{collections::BTreeMap, iter::Iterator, str::from_utf8};

use delegate::delegate;
use serde_bibtex::token::is_balanced;

use crate::error::{InvalidBytesError, RecordDataError};

/// The current version of the binary data format.
pub const fn binary_format_version() -> u8 {
    0
}

/// The size (in bytes) of the version header.
const DATA_HEADER_SIZE: usize = 1;

/// The type of integer used in the header for the BibTeX key.
pub(crate) type KeyHeader = u8;

/// The type of integer used in the header for the BibTeX value.
pub(crate) type ValueHeader = u16;

/// The type of integer used in the BibTeX entry type header.
pub(crate) type EntryTypeHeader = u8;

/// The maximum possible size (in bytes) of a data block.
const MAX_DATA_BLOCK_SIZE: usize = KeyHeader::BITS as usize / 8
    + ValueHeader::BITS as usize / 8
    + KeyHeader::MAX as usize
    + ValueHeader::MAX as usize;

/// The maximum possible size (in bytes) of the BibTeX entry type block.
const MAX_TYPE_BLOCK_SIZE: usize =
    EntryTypeHeader::BITS as usize / 8 + EntryTypeHeader::MAX as usize;

/// The maximum possible size (in bytes) of the vector returned by [`RawRecordData::to_byte_repr`].
pub const DATA_MAX_BYTES: usize = 50_000_000;

/// The maximum number of allowed record fields.
///
/// This number is chosen to be as large as possible while satisfying
/// ```ignore
/// DATA_HEADER_SIZE + MAX_TYPE_BLOCK_SIZE + RECORD_MAX_FIELDS * MAX_DATA_BLOCK_SIZE <= DATA_MAX_BYTES
/// ```
const RECORD_MAX_FIELDS: usize =
    (DATA_MAX_BYTES - DATA_HEADER_SIZE - MAX_TYPE_BLOCK_SIZE) / MAX_DATA_BLOCK_SIZE;

/// This trait represents types which encapsulate the data content of a single BibTeX entry.
pub trait EntryData {
    /// Iterate over `(key, value)` pairs in order.
    fn fields(&self) -> impl Iterator<Item = (&str, &str)>;

    /// Get the `entry_type` as a string slice.
    fn entry_type(&self) -> &str;
}

/// A raw binary representation of the field key and fields of a BibTeX entry.
///
/// This struct is immutable by design. For a mutable version which supports addition and deletion
/// of fields, see [`RecordData`].
///
/// For a description of the binary format, see the [`db`](crate::db) module documentation.
#[derive(Debug, PartialEq, Eq)]
pub struct RawRecordData {
    data: Vec<u8>,
}

impl RawRecordData {
    /// Construct a [`RawRecordData`] from raw bytes without performing any consistency checks.
    ///
    /// # Safety
    /// The caller must ensure that underlying data upholds the requirements of the binary representation.
    pub(super) unsafe fn from_byte_repr_unchecked(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Construct a [`RawRecordData`] from raw bytes, checking that the underlying bytes are valid.
    pub fn from_byte_repr(data: Vec<u8>) -> Result<Self, InvalidBytesError> {
        match data[..] {
            [0, ..] => {
                let mut cursor = Self::check_type(&data, 1)?;
                let mut counter = 0;
                loop {
                    match Self::check_data_block(&data, cursor)? {
                        Some(next_cursor) => {
                            if counter >= RECORD_MAX_FIELDS {
                                break Err(InvalidBytesError::new(cursor, "too many fields"));
                            } else {
                                cursor = next_cursor;
                                counter += 1;
                            }
                        }
                        None => break Ok(unsafe { Self::from_byte_repr_unchecked(data) }),
                    }
                }
            }
            [_, ..] => Err(InvalidBytesError::new(0, "invalid version")),
            [] => Err(InvalidBytesError::new(0, "data was empty")),
        }
    }

    /// Check that the `entry type` block is valid and return the updated cursor position.
    fn check_type(data: &[u8], cursor: usize) -> Result<usize, InvalidBytesError> {
        match data[cursor..] {
            [0, ..] => Err(InvalidBytesError::new(
                cursor,
                "entry type cannot have length zero",
            )),
            [entry_type_len, ..] => {
                let entry_type_start = cursor + 1;
                let entry_type_end = entry_type_start + entry_type_len as usize;
                let entry_type_bytes =
                    data.get(entry_type_start..entry_type_end)
                        .ok_or(InvalidBytesError::new(
                            entry_type_start,
                            "entry type shorter than header",
                        ))?;

                let entry_type = from_utf8(entry_type_bytes).map_err(|e| {
                    InvalidBytesError::new(
                        entry_type_start + e.valid_up_to(),
                        "entry type has invalid utf-8 starting at position",
                    )
                })?;
                if entry_type.chars().any(|ch| !ch.is_ascii_lowercase()) {
                    return Err(InvalidBytesError::new(
                        entry_type_start,
                        "entry type contains chars which are not ascii lowercase",
                    ));
                }
                Ok(entry_type_end)
            }
            _ => Err(InvalidBytesError::new(cursor, "missing entry type")),
        }
    }

    /// Check that a `data block` is valid. If there are no more blocks, return `Ok(None)`;
    /// otherwise, return the updated cursor position.
    fn check_data_block(data: &[u8], cursor: usize) -> Result<Option<usize>, InvalidBytesError> {
        match data[cursor..] {
            [0, _, _, ..] => Err(InvalidBytesError::new(
                cursor,
                "key cannot have length zero",
            )),
            [key_len, value_len_0, value_len_1, ..] => {
                let value_len = u16::from_le_bytes([value_len_0, value_len_1]) as usize;

                let key_block_start = cursor + 3;
                let value_block_start = key_block_start + key_len as usize;
                let value_block_end = value_block_start + value_len;

                let key_bytes =
                    data.get(key_block_start..value_block_start)
                        .ok_or(InvalidBytesError::new(
                            key_block_start,
                            "key block shorter than header",
                        ))?;
                let value_bytes =
                    data.get(value_block_start..value_block_end)
                        .ok_or(InvalidBytesError::new(
                            value_block_start,
                            "value block shorter than header",
                        ))?;

                if !is_balanced(value_bytes) {
                    return Err(InvalidBytesError::new(
                        value_block_start,
                        "value has unbalanced `{}`",
                    ));
                }

                let key = from_utf8(key_bytes).map_err(|e| {
                    InvalidBytesError::new(
                        key_block_start + e.valid_up_to(),
                        "key block has invalid utf-8 starting at position",
                    )
                })?;
                if key.chars().any(|ch| !ch.is_ascii_lowercase()) {
                    return Err(InvalidBytesError::new(
                        key_block_start,
                        "key contains chars which are not ascii lowercase",
                    ));
                }
                let _value = from_utf8(value_bytes).map_err(|e| {
                    InvalidBytesError::new(
                        value_block_start + e.valid_up_to(),
                        "value block has invalid utf-8 starting at position",
                    )
                })?;

                Ok(Some(value_block_end))
            }
            [] => Ok(None),
            _ => Err(InvalidBytesError::new(
                cursor,
                "incomplete data block header",
            )),
        }
    }

    /// The representation as raw bytes.
    pub fn to_byte_repr(&self) -> &[u8] {
        &self.data
    }

    /// Split into the `TYPE` and `DATA` blocks, discarding the header.
    #[inline]
    fn split_blocks(&self) -> (&[u8], &[u8]) {
        let contents = &self.data[DATA_HEADER_SIZE..];
        contents.split_at(contents[0] as usize + 1)
    }
}

impl EntryData for RawRecordData {
    fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        let (_, data_blocks) = self.split_blocks();
        RawRecordFieldsIter {
            remaining: data_blocks,
        }
    }

    fn entry_type(&self) -> &str {
        let (type_block, _) = self.split_blocks();
        from_utf8(&type_block[1..]).unwrap()
    }
}

impl From<&RecordData> for RawRecordData {
    /// Convert a [`RecordData`] into a [`RawRecordData`] for insertion into the database.
    fn from(record_data: &RecordData) -> Self {
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

/// The iterator type for the fields of a [`RawRecordData`]. This cannot be constructed directly;
/// it is constructed implicitly by the [`EntryData::fields`] implementation of [`RawRecordData`].
#[derive(Debug)]
struct RawRecordFieldsIter<'a> {
    remaining: &'a [u8],
}

impl<'a> Iterator for RawRecordFieldsIter<'a> {
    type Item = (&'a str, &'a str);

    /// Iterate over the underlying `(key, value)` blocks.
    ///
    /// # Panics
    /// Panics if the underlying data is malformed.
    fn next(&mut self) -> Option<Self::Item> {
        if !self.remaining.is_empty() {
            let key_len = self.remaining[0] as usize;
            let value_len = u16::from_le_bytes([self.remaining[1], self.remaining[2]]) as usize;
            let tail = &self.remaining[3..];

            let (key, tail) = tail.split_at(key_len);
            let (value, tail) = tail.split_at(value_len);

            self.remaining = tail;

            Some((from_utf8(key).unwrap(), from_utf8(value).unwrap()))
        } else {
            None
        }
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

impl<D: EntryData> From<D> for RecordData {
    fn from(value: D) -> Self {
        let mut new = Self::new_unchecked(value.entry_type().to_owned());
        for (key, value) in value.fields() {
            new.fields.insert(key.to_owned(), value.to_owned());
        }
        new
    }
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
    /// The following rules are checked before insertion. The first one that fails, if any, results
    /// in the corresponding [`RecordDataError`].
    ///
    /// L. RecordData can contain at most [`RECORD_MAX_FIELDS`] entries.
    /// K1. `key` must have length at least `1` and at most [`KeyHeader::MAX`].
    /// K2. `key` must be composed only of ASCII lowercase letters (from [`char::is_ascii_lowercase`]).
    /// V1. `value` must have length at most [`ValueHeader::MAX`].
    /// V2. `value` must satisfy the balanced `{}` rule (from [`serde_bibtex::token::is_balanced`]).
    pub fn check_and_insert(
        &mut self,
        key: String,
        value: String,
    ) -> Result<Option<String>, RecordDataError> {
        // Condition L
        if self.fields.len() >= RECORD_MAX_FIELDS && !self.fields.contains_key(&key) {
            return Err(RecordDataError::RecordDataFull);
        }

        // Conditions K1 and K2
        Self::check_key(&key)?;

        // Conditions V1 and V2
        Self::check_value(&value)?;

        Ok(self.fields.insert(key, value))
    }

    /// Merge data from `other`, ignoring fields that already exist in `self`.
    pub fn merge_or_skip<D: EntryData>(&mut self, other: D) -> Result<(), RecordDataError> {
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
        other: D,
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

    pub fn get<Q>(&self, key: &Q) -> Option<&str>
    where
        String: std::borrow::Borrow<Q>,
        Q: Ord,
    {
        self.fields.get(key).map(String::as_str)
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        String: std::borrow::Borrow<Q>,
        Q: Ord,
    {
        self.fields.contains_key(key)
    }

    pub fn remove<Q>(&mut self, key: &Q) -> Option<String>
    where
        String: std::borrow::Borrow<Q>,
        Q: Ord,
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

impl EntryData for &RecordData {
    fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    fn entry_type(&self) -> &str {
        &self.entry_type
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Check that conversion into the raw form and back results in identical data.
    #[test]
    fn test_data_round_trip() {
        let mut record_data = RecordData::try_new("article".into()).unwrap();
        record_data
            .check_and_insert("year".into(), "2024".into())
            .unwrap();
        record_data
            .check_and_insert("title".into(), "A title".into())
            .unwrap();
        record_data
            .check_and_insert("field".into(), "".into())
            .unwrap();
        record_data
            .check_and_insert("a".repeat(255), "🍄".into())
            .unwrap();
        record_data
            .check_and_insert("a".into(), "b".repeat(65_535))
            .unwrap();

        let raw_data = RawRecordData::from(&record_data);

        let mut record_data_clone = RecordData::try_new(raw_data.entry_type().into()).unwrap();

        for (key, value) in raw_data.fields() {
            record_data_clone
                .check_and_insert(key.into(), value.into())
                .unwrap();
        }

        assert_eq!(record_data, record_data_clone);
        assert_eq!(
            raw_data.to_byte_repr(),
            RawRecordData::from(&record_data_clone).to_byte_repr()
        );
    }

    #[test]
    fn test_insert_len() {
        let mut record_data = RecordData::try_new("a".into()).unwrap();

        assert_eq!(
            record_data.check_and_insert("a".repeat(256), "".into()),
            Err(RecordDataError::KeyInvalidLength(256))
        );

        assert_eq!(
            record_data.check_and_insert("a".into(), "🍄".repeat(20_000)),
            Err(RecordDataError::ValueInvalidLength(80_000))
        );

        assert_eq!(
            record_data.check_and_insert("".into(), "".into()),
            Err(RecordDataError::KeyInvalidLength(0))
        );

        assert!(record_data
            .check_and_insert("a".repeat(255), "".into())
            .is_ok(),);
    }

    #[test]
    fn test_format_manual() {
        let mut record_data = RecordData::try_new("article".into()).unwrap();
        record_data
            .check_and_insert("year".into(), "2023".into())
            .unwrap();
        record_data
            .check_and_insert("title".into(), "The Title".into())
            .unwrap();

        let data = RawRecordData::from(&record_data);
        let expected = vec![
            0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 5, 9, 0, b't', b'i', b't', b'l', b'e',
            b'T', b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e', b'a', b'r',
            b'2', b'0', b'2', b'3',
        ];

        assert_eq!(expected, data.to_byte_repr());
    }

    #[test]
    fn test_validate_data_ok() {
        for data in [
            // usual example
            vec![
                0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 5, 9, 0, b't', b'i', b't', b'l',
                b'e', b'T', b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e',
                b'a', b'r', b'2', b'0', b'2', b'3',
            ],
            // no keys is OK
            vec![0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e'],
            // field value can have length 0
            vec![0, 1, b'a', 1, 0, 0, b'b'],
        ] {
            assert!(RawRecordData::from_byte_repr(data).is_ok());
        }
    }

    #[test]
    fn test_validate_data_err() {
        // invalid version
        let malformed_data = vec![
            1, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 5, 9, 0, b't', b'i', b't', b'l', b'e',
            b'T', b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e', b'a', b'r',
            b'2', b'0', b'2', b'3',
        ];
        let parsed = RawRecordData::from_byte_repr(malformed_data);
        assert!(matches!(
            parsed,
            Err(InvalidBytesError {
                position: 0,
                message: "invalid version"
            })
        ));

        // entry type is not valid utf-8
        let malformed_data = vec![
            0, 7, b'a', b'r', b't', 255, b'c', b'l', b'e', 5, 9, 0, b't', b'i', b't', b'l', b'e',
            b'T', b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e', b'a', b'r',
            b'2', b'0', b'2', b'3',
        ];
        let parsed = RawRecordData::from_byte_repr(malformed_data);
        assert!(matches!(parsed, Err(InvalidBytesError { position: 5, .. })));

        // bad length header
        let malformed_data = vec![
            0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 5, 100, 0, b't', b'i', b't', b'l',
            b'e', b'T', b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e', b'a',
            b'r', b'2', b'0', b'2', b'3',
        ];
        let parsed = RawRecordData::from_byte_repr(malformed_data);
        assert!(matches!(
            parsed,
            Err(InvalidBytesError {
                position: 17,
                message: "value block shorter than header"
            })
        ));

        // trailing bytes
        let malformed_data = vec![0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 1];
        let parsed = RawRecordData::from_byte_repr(malformed_data);
        assert!(parsed.is_err());

        // entry type cannot have length 0
        let malformed_data = vec![0, 0];
        let parsed = RawRecordData::from_byte_repr(malformed_data);
        assert!(parsed.is_err());

        // field key cannot have length 0
        let malformed_data = vec![0, 1, b'a', 0, 0, 0];
        let parsed = RawRecordData::from_byte_repr(malformed_data);
        assert!(parsed.is_err());
    }

    #[test]
    fn test_data_err_insert() {
        assert_eq!(
            RecordData::try_new("".into()),
            Err(RecordDataError::EntryTypeInvalidLength(0)),
        );

        assert_eq!(
            RecordData::try_new("b".repeat(300)),
            Err(RecordDataError::EntryTypeInvalidLength(300)),
        );

        assert_eq!(
            RecordData::try_new("🍄".into()),
            Err(RecordDataError::EntryTypeNotAsciiLowercase),
        );

        let mut record_data = RecordData::try_new("a".into()).unwrap();

        assert_eq!(
            record_data.check_and_insert("BAD".into(), "".into()),
            Err(RecordDataError::KeyNotAsciiLowercase)
        );

        assert_eq!(
            record_data.check_and_insert("".into(), "".into()),
            Err(RecordDataError::KeyInvalidLength(0))
        );

        assert!(record_data.is_empty());
    }
}
