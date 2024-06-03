use std::fmt;

use delegate::delegate;

use crate::db::Data;

/// A single regular entry in a BibTex bibliography.
#[derive(Debug)]
pub struct Entry<D: Data> {
    key: String,
    record_data: D,
}

impl<D: Data> Entry<D> {
    pub fn new<T: Into<String>>(key: T, record_data: D) -> Self {
        Self {
            key: key.into(),
            record_data,
        }
    }

    pub fn key(&self) -> &str {
        &self.key
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

impl<D: Data> fmt::Display for Entry<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}{{{},", self.record_data.entry_type(), self.key)?;
        for (key, value) in self.record_data.fields() {
            write_biblatex_row(f, key, value)?;
        }
        write!(f, "\n}}")?;

        Ok(())
    }
}
