//! ## Description of the internal binary format
//! We use a custom internal binary format to represent the data associated with each bibTex entry.
//!
//! The first byte is the version.
//! Depending on the version, the format is as follows.
//!
//! ### Version 0
//! The data is stored as a sequence of blocks.
//! ```txt
//! HEADER, TYPE, DATA1, DATA2, ...
//! ```
//! The `HEADER` consists of
//! ```txt
//! version: u8,
//! ```
//! and the `TYPE` consists of
//! ```txt
//! [entry_type_len: u8, entry_type: [u8..]]
//! ```
//! Here, `entry_type_len` is the length of `entry_type`, which has length at most [`u8::MAX`].
//! Then, each block `DATA` is of the form
//! ```txt
//! [key_len: u8, value_len: u16, key: [u8..], value: [u8..]]
//! ```
//! where `key_len` is the length of the first `key` segment, and the `value_len` is
//! the length of the `value` segment. Necessarily, `key` and `value` have lengths at
//! most [`u8::MAX`] and [`u16::MAX`] respectively.
//!
//! `value_len` is encoded in little endian format.
//!
//! The `DATA...` are sorted by `key` and each `key` and `entry_type` must be ASCII lowercase. The
//! `entry_type` can be any valid UTF-8.
//!
//! For example we would serialize
//! ```bib
//! @article{...,
//!   Year = {192},
//!   Title = {The Title},
//! }
//! ```
//! as
//! ```
//! # let mut record_data = RecordData::try_new("article".into()).unwrap();
//! # record_data.check_and_insert("year".into(), "2023".into()).unwrap();
//! # record_data
//! #     .check_and_insert("title".into(), "The Title".into())
//! #     .unwrap();
//! # let byte_repr = RawEntryData::from(&record_data).into_byte_repr();
//! let expected = vec![
//!     0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 5, 9, 0, b't', b'i', b't', b'l', b'e',
//!     b'T', b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e', b'a', b'r',
//!     b'2', b'0', b'2', b'3',
//! ];
//! # assert_eq!(expected_byte_repr, byte_repr);
//! ```

use crate::entry::RawEntryData;

pub enum RawRecordData<T> {
    Entry(RawEntryData<T>),
    Merged(String),
    Deleted,
}

impl<T: AsRef<[u8]>> RawRecordData<T> {
    pub(crate) fn from_byte_repr_unchecked(data: T) -> Self {
        match data.as_ref() {
            [] => Self::Deleted,
            [0, ..] => Self::Entry(RawEntryData::from_byte_repr_unchecked(data)),
            [b'@', ..] => Self::Merged(std::str::from_utf8(data.as_ref()).unwrap().into()),
            _ => panic!("Database contains malformed data."),
        }
    }

    /// The representation as raw bytes.
    pub fn to_byte_repr(&self) -> &[u8] {
        match self {
            Self::Entry(raw_entry_data) => raw_entry_data.to_byte_repr(),
            Self::Deleted => &[],
            Self::Merged(s) => s.as_ref(),
        }
    }
}
