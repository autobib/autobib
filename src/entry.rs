use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use delegate::delegate;
use serde::Deserialize;
use serde_bibtex::de::Deserializer;

use crate::db::{EntryData, RawRecordData, RecordData};
use crate::error::RecordDataError;

/// A single regular entry in a BibTeX bibliography.
#[derive(Debug, PartialEq)]
pub struct Entry<D: EntryData> {
    key: String,
    record_data: D,
}

impl<D: EntryData> Entry<D> {
    pub fn new<T: Into<String>>(key: T, record_data: D) -> Self {
        Self {
            key: key.into(),
            record_data,
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn data(&self) -> &D {
        &self.record_data
    }

    pub fn into_parts(self) -> (String, D) {
        (self.key, self.record_data)
    }

    delegate! {
        to self.record_data {
            pub fn fields(&self) -> impl Iterator<Item = (&str, &str)>;
            pub fn entry_type(&self) -> &str;
        }

    }
}

/// A temporary struct used as an intermediate deserialization target.
#[derive(Debug, PartialEq, Deserialize)]
struct Contents {
    entry_type: String,
    entry_key: String,
    fields: BTreeMap<String, String>,
}

impl FromStr for Entry<RawRecordData> {
    type Err = RecordDataError;

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

            Ok(Entry::new(entry_key, (&record_data).into()))
        } else {
            Err(Self::Err::BibtexReadError)
        }
    }
}

fn write_biblatex_row(f: &mut fmt::Formatter<'_>, key: &str, value: &str) -> fmt::Result {
    write!(f, "\n  {key} = {{{value}}},")
}

impl<D: EntryData> fmt::Display for Entry<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}{{{},", self.record_data.entry_type(), self.key)?;
        for (key, value) in self.record_data.fields() {
            write_biblatex_row(f, key, value)?;
        }
        write!(f, "\n}}")?;

        Ok(())
    }
}
