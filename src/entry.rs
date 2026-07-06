mod data;
mod deserialize;

use serde_bibtex::{MacroDictionary, de::Deserializer};
use std::{fmt, io, str::FromStr};

pub use self::data::{
    AsEntryData, ConflictResolved, EntryData, EntryEditCommand, EntryKey, EntryType, FieldKey,
    FieldValue, MutableEntryData, RawEntryData, RawRecordFieldsIter, SetFieldCommand,
};
pub(crate) use self::data::{EntryTypeHeader, KeyHeader, ValueHeader};

use crate::error::BibtexDataError;

/// A single regular entry in a BibTeX bibliography.
#[derive(Debug, PartialEq)]
pub struct Entry<D, S = String> {
    pub key: EntryKey<S>,
    pub record_data: D,
}

impl<D, S> Entry<D, S> {
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
}

trait EntryWrite {
    type Error;

    fn write_fmt(&mut self, args: std::fmt::Arguments<'_>) -> Result<(), Self::Error>;
}

impl<W: EntryWrite> EntryWrite for &mut W {
    type Error = W::Error;

    fn write_fmt(&mut self, args: std::fmt::Arguments<'_>) -> Result<(), Self::Error> {
        (*self).write_fmt(args)
    }
}

struct IOWriteWrap<W>(W);

impl<W: io::Write> EntryWrite for IOWriteWrap<W> {
    type Error = io::Error;

    fn write_fmt(&mut self, args: std::fmt::Arguments<'_>) -> Result<(), Self::Error> {
        self.0.write_fmt(args)
    }
}

struct FmtWriteWrap<W>(W);

impl<W: fmt::Write> EntryWrite for FmtWriteWrap<W> {
    type Error = fmt::Error;

    fn write_fmt(&mut self, args: std::fmt::Arguments<'_>) -> Result<(), Self::Error> {
        self.0.write_fmt(args)
    }
}

impl<D: AsEntryData, S: AsRef<str>> Entry<D, S> {
    fn write_generic<W: EntryWrite>(&self, mut writer: W) -> Result<(), W::Error> {
        let tmp = self.record_data.as_entry_data();
        let (entry_type, fields) = tmp.entry_type_and_fields();
        writeln!(writer, "@{}{{{},", entry_type, self.key.as_ref())?;
        for (key, value) in fields {
            writeln!(writer, "  {key} = {{{value}}},")?;
        }
        write!(writer, "}}")
    }

    pub fn write_io<W: io::Write>(&self, writer: W) -> Result<(), io::Error> {
        self.write_generic(IOWriteWrap(writer))
    }

    pub fn write_fmt<W: fmt::Write>(&self, writer: W) -> Result<(), fmt::Error> {
        self.write_generic(FmtWriteWrap(writer))
    }
}
impl<D: AsEntryData, S: AsRef<str>> fmt::Display for Entry<D, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_fmt(f)
    }
}

pub fn entries_to_bibtex<'a, W, E, D, S>(mut writer: W, entries: E) -> Result<(), io::Error>
where
    W: io::Write,
    D: AsEntryData + 'a,
    S: AsRef<str> + 'a,
    E: IntoIterator<Item = &'a Entry<D, S>>,
{
    let mut first = true;
    for entry in entries {
        if first {
            first = false;
        } else {
            write!(writer, "\n\n")?;
        }

        entry.write_io(&mut writer)?;
    }
    writeln!(writer)
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
