use std::{fmt, io, str::FromStr};

use autobib_entry::{
    data::{EntryData, MutableEntryData},
    error::DataError,
    v0::ArchivedEntryData,
};
use serde::Deserialize;
use serde_bibtex::{MacroDictionary, de::Deserializer, token::check_entry_key};

use crate::error::BibtexDataError;

/// A validated entry key (e.g. "key" in `@book{key, ..}`) which satisfies the following
/// requirements:
///
/// 1. has length at least `1`
/// 2. composed only of ASCII printable characters except `{}(),=\\#%\"`, or non-ASCII UTF-8.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub struct EntryKey<S = String>(pub(in crate::entry) S);

impl<S: AsRef<str>> EntryKey<S> {
    #[inline]
    pub fn try_new(s: S) -> Result<Self, DataError> {
        let entry_key = s.as_ref();

        check_entry_key(entry_key)?;

        Ok(Self(s))
    }

    pub fn is_placeholder(&self) -> bool {
        self.0.as_ref().starts_with(':')
    }
}

impl<T: From<&'static str>> EntryKey<T> {
    /// A placeholder value used for displaying keys which are not valid bibtex.
    pub fn placeholder() -> Self {
        Self("::".into())
    }
}

impl<S: AsRef<str>> AsRef<str> for EntryKey<S> {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

impl<S: AsRef<str>> ::std::fmt::Display for EntryKey<S> {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.0.as_ref())
    }
}

impl<S: AsRef<str>> PartialEq<str> for EntryKey<S> {
    fn eq(&self, other: &str) -> bool {
        self.0.as_ref().eq(other)
    }
}

impl ::std::str::FromStr for EntryKey {
    type Err = DataError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_new(s.into())
    }
}

/// A single regular entry in a BibTeX bibliography.
#[derive(Debug, PartialEq, Deserialize)]
pub struct BibtexEntry<D = Box<ArchivedEntryData>, S = String> {
    #[serde(rename = "entry_key")]
    pub key: EntryKey<S>,
    #[serde(flatten)]
    pub record_data: D,
}

impl<D, S> BibtexEntry<D, S> {
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

impl<D: EntryData, S: AsRef<str>> BibtexEntry<D, S> {
    fn write_generic<W: EntryWrite>(&self, mut writer: W) -> Result<(), W::Error> {
        let entry_type = self.record_data.entry_type();
        let fields = self.record_data.fields();
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
impl<D: EntryData, S: AsRef<str>> fmt::Display for BibtexEntry<D, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_fmt(f)
    }
}

pub fn entries_to_bibtex<'a, W, E, D, S>(mut writer: W, entries: E) -> Result<(), io::Error>
where
    W: io::Write,
    D: EntryData + 'a,
    S: AsRef<str> + 'a,
    E: IntoIterator<Item = &'a BibtexEntry<D, S>>,
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
) -> impl Iterator<Item = Result<BibtexEntry<MutableEntryData>, BibtexDataError>> + use<'_> {
    let mut dct = MacroDictionary::default();
    dct.set_month_macros();
    Deserializer::from_slice_with_macros(bibtex, dct)
        .into_iter_regular_entry::<BibtexEntry<MutableEntryData>>()
        .map(|res_entry| res_entry.map_err(Into::into))
}

impl FromStr for BibtexEntry<MutableEntryData> {
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
