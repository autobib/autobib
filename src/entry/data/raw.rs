use std::str::from_utf8;

use serde_bibtex::token::is_balanced;

use super::{validate_ascii_identifier, EntryData};
use crate::error::InvalidBytesError;

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

/// A raw binary representation of the field key and fields of a BibTeX entry.
///
/// This struct is immutable by design. For a mutable version which supports addition and deletion
/// of fields, see [`RecordData`](super::RecordData).
///
/// For a description of the binary format, see the [`db`](crate::db) module documentation.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RawRecordData {
    data: Vec<u8>,
}

impl RawRecordData {
    /// Construct a [`RawRecordData`] from raw bytes without performing any consistency checks.
    ///
    /// # Safety
    /// The caller must ensure that underlying data upholds the requirements of the binary representation.
    pub(crate) unsafe fn from_byte_repr_unchecked(data: Vec<u8>) -> Self {
        Self { data }
    }

    pub fn from_entry_data<D: EntryData>(entry_data: &D) -> Self {
        let mut data = Vec::with_capacity(entry_data.raw_len());

        data.push(binary_format_version());

        let entry_type = entry_data.entry_type();
        let entry_type_len = EntryTypeHeader::try_from(entry_type.len()).unwrap();
        data.push(entry_type_len);
        data.extend(entry_type.as_bytes());

        for (key, value) in entry_data.fields() {
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

    /// The representation as raw bytes.
    pub fn to_byte_repr(&self) -> &[u8] {
        &self.data
    }

    /// Construct a [`RawRecordData`] from raw bytes, checking that the underlying bytes are valid.
    pub fn from_byte_repr(data: Vec<u8>) -> Result<Self, InvalidBytesError> {
        match data[..] {
            [0, ..] => {
                let mut cursor = Self::check_type(&data, 1)?;
                loop {
                    match Self::check_data_block(&data, cursor)? {
                        Some(next_cursor) => {
                            cursor = next_cursor;
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

                if validate_ascii_identifier(entry_type_bytes).is_err() {
                    return Err(InvalidBytesError::new(
                        entry_type_start,
                        "entry type contains non-ASCII chararacters or invalid ASCII characters",
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

                if validate_ascii_identifier(key_bytes).is_err() {
                    return Err(InvalidBytesError::new(
                        key_block_start,
                        "field key contains non-ASCII chararacters or invalid ASCII characters",
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

    /// Split into the `TYPE` and `DATA` blocks, discarding the header.
    #[inline]
    fn split_blocks(&self) -> (&[u8], &[u8]) {
        let contents = &self.data[DATA_HEADER_SIZE..];
        contents.split_at(contents[0] as usize + 1)
    }
}

/// The iterator type for the fields of a [`RawRecordData`]. This cannot be constructed directly;
/// it is constructed implicitly by the [`EntryData::fields`] implementation of [`RawRecordData`].
#[derive(Debug)]
pub(super) struct RawRecordFieldsIter<'a> {
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

unsafe impl EntryData for RawRecordData {
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

    fn raw_len(&self) -> usize {
        self.data.len()
    }
}
