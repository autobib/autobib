mod data;
mod deserialize;

use std::{fmt, str::FromStr};

use delegate::delegate;
use serde::ser::{Serialize, SerializeSeq, SerializeStruct, Serializer};
use serde_bibtex::{MacroDictionary, de::Deserializer, to_string_unchecked};

pub use self::data::{
    BorrowedEntryData, ConflictResolved, EntryData, EntryKey, EntryType, FieldKey, FieldValue,
    RawRecordData, RecordData,
};
pub(crate) use self::data::{EntryTypeHeader, KeyHeader, ValueHeader};

use crate::error::BibtexDataError;

/// A single regular entry in a BibTeX bibliography.
#[derive(Debug, PartialEq)]
pub struct Entry<D> {
    pub key: EntryKey<String>,
    pub record_data: D,
}

impl<D: EntryData> Entry<D> {
    /// Create a new entry with the provided key and record data.
    pub fn new(key: EntryKey<String>, record_data: D) -> Self {
        Self { key, record_data }
    }

    pub fn key(&self) -> &EntryKey<String> {
        &self.key
    }

    pub fn data(&self) -> &D {
        &self.record_data
    }

    delegate! {
        to self.record_data {
            pub fn fields(&self) -> impl Iterator<Item = (&str, &str)>;
            pub fn entry_type(&self) -> &str;
        }
    }
}

struct RecordDataWrapper<D>(D);

impl<D: EntryData> Serialize for RecordDataWrapper<&'_ D> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_seq(None)?;
        for (key, value) in self.0.fields() {
            state.serialize_element(&(key, value))?;
        }
        state.end()
    }
}

impl<D: EntryData> Serialize for Entry<D> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("Entry", 3)?;
        state.serialize_field("entry_type", &self.entry_type())?;
        state.serialize_field("entry_key", &self.key.as_ref())?;
        state.serialize_field("fields", &RecordDataWrapper(&self.record_data))?;
        state.end()
    }
}

pub fn entries_from_bibtex(
    bibtex: &[u8],
) -> impl Iterator<Item = Result<Entry<RecordData>, BibtexDataError>> + use<'_> {
    let mut dct = MacroDictionary::default();
    dct.set_month_macros();
    Deserializer::from_slice_with_macros(bibtex, dct)
        .into_iter_regular_entry::<Entry<RecordData>>()
        .map(|res_entry| res_entry.map_err(Into::into))
}

impl FromStr for Entry<RecordData> {
    type Err = BibtexDataError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut de_iter = Deserializer::from_str(s).into_iter_regular_entry::<Self>();

        match de_iter.next() {
            Some(Ok(entry)) => {
                if de_iter.next().is_none() {
                    Ok(entry)
                } else {
                    Err(Self::Err::BibtexMultipleEntries)
                }
            }
            Some(Err(err)) => Err(Self::Err::BibtexParseError(err)),
            None => Err(Self::Err::Empty),
        }
    }
}

impl<D: EntryData> fmt::Display for Entry<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY: the RecordData::try_new and RecordData::check_and_insert methods only accept
        //         entry types and field keys which satisfy stricter requirements than the
        //         serde_bibtex syntax
        let buffer = to_string_unchecked(&[self]).expect("serialization should not fail");
        f.write_str(&buffer)
    }
}
