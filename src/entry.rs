use std::{collections::BTreeMap, fmt, str::FromStr};

use delegate::delegate;
use serde::{
    ser::{Serialize, SerializeSeq, SerializeStruct, Serializer},
    Deserialize,
};
use serde_bibtex::{de::Deserializer, to_string_unchecked, token::EntryKey};

use crate::{
    db::{EntryData, RawRecordData, RecordData},
    error::BibtexDataError,
};

/// A single regular entry in a BibTeX bibliography.
#[derive(Debug, PartialEq)]
pub struct Entry<D: EntryData> {
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

/// A temporary struct used as an intermediate deserialization target, which can be easily
/// converted into an [`Entry`].
#[derive(Debug, PartialEq, Deserialize)]
struct Contents {
    entry_type: String,
    entry_key: String,
    fields: BTreeMap<String, String>,
}

impl FromStr for Entry<RawRecordData> {
    type Err = BibtexDataError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut de_iter = Deserializer::from_str(s).into_iter_regular_entry();

        if let Some(Ok(Contents {
            mut entry_type,
            entry_key,
            mut fields,
        })) = de_iter.next()
        {
            if de_iter.next().is_none() {
                entry_type.make_ascii_lowercase();
                let mut record_data = RecordData::try_new(entry_type)?;
                while let Some((mut key, val)) = fields.pop_first() {
                    key.make_ascii_lowercase();
                    record_data.check_and_insert(key, val)?;
                }

                // SAFETY: the Deserializer implementation only accepts the entry if the entry key is
                //         valid.
                Ok(Entry::new(
                    EntryKey::new(entry_key).unwrap(),
                    (&record_data).into(),
                ))
            } else {
                Err(Self::Err::BibtexMultipleEntries)
            }
        } else {
            Err(Self::Err::BibtexParseError)
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
