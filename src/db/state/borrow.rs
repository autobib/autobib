//! # Borrow record data
//!
//! This module implements some abstractions over data which borrows from some row. This is mainly
//! useful when you want to do some sort of computation on a row, but you don't actually need to
//! own all of the data, so you can save on some allocations by using the types here.
use chrono::{DateTime, Local};
use rusqlite::{Row, types::ValueRef};

use super::{ArbitraryData, RecordRow};
use crate::{RawEntryData, RemoteId};

/// Equivalent to an [`ArbitraryData`], but borrows all of its data.
#[derive(Debug)]
pub enum ArbitraryDataRef<'r> {
    /// Entry data.
    Entry(RawEntryData<&'r [u8]>),
    /// Deleted data.
    Deleted(Option<RemoteId<&'r str>>),
    /// Void data.
    Void,
}

impl ArbitraryData {
    /// Get a reference to the data in this struct.
    pub fn as_deref(&self) -> ArbitraryDataRef<'_> {
        match self {
            Self::Entry(raw_entry_data) => ArbitraryDataRef::Entry(raw_entry_data.as_deref()),
            Self::Deleted(replacement) => {
                ArbitraryDataRef::Deleted(replacement.as_ref().map(RemoteId::as_deref))
            }
            Self::Void => ArbitraryDataRef::Void,
        }
    }
}

impl<'r> ArbitraryDataRef<'r> {
    /// Borrow from bytes with the provided variant, interpreted according to the variant.
    pub(in crate::db) fn from_borrowed_bytes_and_variant(bytes: &'r [u8], variant: i64) -> Self {
        match variant {
            0 => Self::Entry(RawEntryData::from_byte_repr_unchecked(bytes)),
            1 => Self::Deleted(if bytes.is_empty() {
                None
            } else {
                Some(RemoteId::from_string_unchecked(
                    std::str::from_utf8(bytes).expect(
                "Invalid database: 'data' column for deleted row contains non-UTF8 blob data.",
                        ),
                ))
            }),
            2 => Self::Void,
            _ => panic!("Unexpected 'Records' table row variant: expected entry or deleted data."),
        }
    }
}

impl<'r> RecordRow<ArbitraryDataRef<'r>, &'r str> {
    /// Load from a row in the 'Records' table. The query which produced the row must contain the following columns:
    ///
    /// - `record_id`
    /// - `modified`
    /// - `data`
    /// - `variant`
    pub(in crate::db) fn borrow_from_row_unchecked(row: &'r Row<'_>) -> Self {
        let ValueRef::Text(record_id) = row.get_ref_unwrap("record_id") else {
            panic!("Expected 'record_id' column to be of type TEXT");
        };
        let ValueRef::Blob(data_bytes) = row.get_ref_unwrap("data") else {
            panic!("Expected 'data' column to be of type BLOB");
        };
        let ValueRef::Integer(variant) = row.get_ref_unwrap("variant") else {
            panic!("Expected 'variant' column to be of type INTEGER");
        };
        let modified: DateTime<Local> = row.get_unwrap("modified");
        let data = ArbitraryDataRef::from_borrowed_bytes_and_variant(data_bytes, variant);
        let canonical = RemoteId::from_string_unchecked(std::str::from_utf8(record_id).unwrap());
        Self {
            data,
            modified,
            canonical,
        }
    }
}
