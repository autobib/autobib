mod data;
mod deserialize;

use std::{fmt, io, str::FromStr};

use delegate::delegate;
use serde::ser::{Serialize, SerializeSeq, SerializeStruct, Serializer};
use serde_bibtex::{MacroDictionary, de::Deserializer, ser::Formatter};

pub use self::data::{
    BorrowedEntryData, ConflictResolved, EntryData, EntryEditCommand, EntryKey, EntryType,
    FieldKey, FieldValue, MutableEntryData, RawEntryData, RawRecordFieldsIter, SetFieldCommand,
};
pub(crate) use self::data::{EntryTypeHeader, KeyHeader, ValueHeader};

use crate::error::BibtexDataError;

/// A single regular entry in a BibTeX bibliography.
#[derive(Debug, PartialEq)]
pub struct Entry<D, S = String> {
    pub key: EntryKey<S>,
    pub record_data: D,
}

impl<D: EntryData, S> Entry<D, S> {
    /// Create a new entry with the provided key and record data.
    pub fn new(key: EntryKey<S>, record_data: D) -> Self {
        Self { key, record_data }
    }

    pub fn key(&self) -> &EntryKey<S> {
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

impl<D: EntryData, S: AsRef<str>> Serialize for Entry<D, S> {
    fn serialize<T>(&self, serializer: T) -> Result<T::Ok, T::Error>
    where
        T: Serializer,
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
) -> impl Iterator<Item = Result<Entry<MutableEntryData>, BibtexDataError>> + use<'_> {
    let mut dct = MacroDictionary::default();
    dct.set_month_macros();
    Deserializer::from_slice_with_macros(bibtex, dct)
        .into_iter_regular_entry::<Entry<MutableEntryData>>()
        .map(|res_entry| res_entry.map_err(Into::into))
}

impl FromStr for Entry<MutableEntryData> {
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

impl<D: EntryData, S: AsRef<str>> fmt::Display for Entry<D, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct FormatterWriter<'a, 'b> {
            formatter: &'a mut fmt::Formatter<'b>,
            failed: bool,
        }

        impl io::Write for FormatterWriter<'_, '_> {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                // SAFETY: serde_bibtex only emits calls which are valid strings
                let s = unsafe { std::str::from_utf8_unchecked(buf) };
                if self.formatter.write_str(s).is_err() {
                    self.failed = true;
                    return Err(io::Error::other(fmt::Error));
                }
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        struct NoTrailingNewlineFormatter;

        impl Formatter for NoTrailingNewlineFormatter {
            fn write_bibliography_end<W>(&mut self, _: &mut W) -> io::Result<()>
            where
                W: ?Sized + io::Write,
            {
                Ok(())
            }
        }

        // SAFETY: the RecordData::try_new and RecordData::check_and_insert methods only accept
        //         entry types and field keys which satisfy stricter requirements than the
        //         serde_bibtex syntax
        let mut writer = FormatterWriter {
            formatter: f,
            failed: false,
        };
        let mut ser = serde_bibtex::ser::Serializer::new_with_formatter(
            &mut writer,
            NoTrailingNewlineFormatter,
        );
        match [self].serialize(&mut ser) {
            Ok(()) => Ok(()),
            Err(_) if writer.failed => Err(fmt::Error),
            Err(err) => panic!("serialization should not fail: {err}"),
        }
    }
}
