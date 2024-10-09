use std::{collections::BTreeMap, fmt, str::FromStr};

use delegate::delegate;
use serde::{
    ser::{Serialize, SerializeSeq, SerializeStruct, Serializer},
    Deserialize,
};
use serde_bibtex::{de::Deserializer, to_string_unchecked, validate::is_entry_key};

use crate::{
    db::{EntryData, RawRecordData, RecordData},
    error::BibTeXError,
};

/// A single regular entry in a BibTeX bibliography.
#[derive(Debug, PartialEq)]
pub struct Entry<D: EntryData> {
    pub key: String,
    pub record_data: D,
}

impl<D: EntryData> Entry<D> {
    /// Create a new entry data with the provided key.
    ///
    /// # Errors
    /// This method will fail if the key contains characters which are invalid BibTeX entry key
    /// characters, as accepted by the [`serde_bibtex::validate::is_entry_key`] method.
    pub fn try_new(key: String, record_data: D) -> Result<Self, BibTeXError> {
        if is_entry_key(&key) {
            Ok(Self::new_unchecked(key, record_data))
        } else {
            Err(BibTeXError::InvalidKey(key))
        }
    }

    /// Create a new entry data with the provided key.
    ///
    /// # Safety
    /// The caller is required to guarantee that the key does not contain any characters which are
    /// invalid BibTeX entry key characters, as accepted by the [`serde_bibtex::validate::is_entry_key`] method.
    pub(crate) fn new_unchecked(key: String, record_data: D) -> Self {
        Self { key, record_data }
    }

    pub fn key(&self) -> &str {
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

impl<'a, D: EntryData> Serialize for RecordDataWrapper<&'a D> {
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
        state.serialize_field("entry_key", &self.key)?;
        state.serialize_field("fields", &RecordDataWrapper(&self.record_data))?;
        state.end()
    }
}

/// A temporary struct used as an intermediate deserialization target, which can be easily
/// converted into an [`Entry`].
#[derive(Debug, PartialEq, Deserialize)]
struct Contents {
    entry_type: String,
    entry_key: String,
    fields: BTreeMap<String, String>,
}

impl FromStr for Entry<RawRecordData> {
    type Err = BibTeXError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(Ok(Contents {
            entry_type,
            entry_key,
            mut fields,
        })) = Deserializer::from_str(s).into_iter_regular_entry().next()
        {
            let mut record_data = RecordData::try_new(entry_type)?;
            while let Some((key, val)) = fields.pop_first() {
                record_data.try_insert(key, val)?;
            }

            // SAFETY: the Deserializer implementation only accepts the entry if the entry key is
            //         valid.
            Ok(Entry::new_unchecked(entry_key, (&record_data).into()))
        } else {
            Err(Self::Err::BibtexParseError)
        }
    }
}

impl<D: EntryData> fmt::Display for Entry<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY: the RecordData::try_new and RecordData::try_insert methods only accept
        //         entry types and field keys which satisfy stricter requirements than the
        //         serde_bibtex syntax
        let buffer = to_string_unchecked(&[self]).expect("serialization should not fail");
        f.write_str(&buffer)
    }
}
