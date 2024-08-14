use std::fmt;

use delegate::delegate;

use crate::db::EntryData;

/// A single regular entry in a BibTeX bibliography.
#[derive(Debug)]
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

    delegate! {
        to self.record_data {
            pub fn fields(&self) -> impl Iterator<Item = (&str, &str)>;
            pub fn entry_type(&self) -> &str;
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
