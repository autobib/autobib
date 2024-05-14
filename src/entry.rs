use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};
use serde_aux::prelude::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct KeyedEntry {
    pub key: String,
    pub contents: Entry,
}

impl Display for KeyedEntry {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "@{}{{{},",
            self.contents.entry_type.to_lowercase(),
            self.key
        )?;
        write!(f, "{}", self.contents.fields)?;
        write!(f, "\n}}")
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub entry_type: String,
    #[serde(deserialize_with = "deserialize_struct_case_insensitive")]
    pub fields: Fields,
}

impl Entry {
    pub fn add_key<T: Into<String>>(self, key: T) -> KeyedEntry {
        KeyedEntry {
            key: key.into(),
            contents: self,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Fields {
    pub title: Option<String>,
    pub author: Option<String>,
    pub journal: Option<String>,
    pub volume: Option<String>,
    pub pages: Option<String>,
    pub year: Option<String>,
}

impl Fields {
    fn write_biblatex_row(
        f: &mut Formatter<'_>,
        field_name: &str,
        field_value: &Option<String>,
    ) -> std::fmt::Result {
        match field_value {
            Some(field_value_string) => write!(f, "\n  {field_name} = {{{field_value_string}}},"),
            None => Ok(()),
        }
    }
}

impl Display for Fields {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Self::write_biblatex_row(f, "title", &self.title)?;
        Self::write_biblatex_row(f, "journal", &self.journal)?;
        Self::write_biblatex_row(f, "volume", &self.volume)?;
        Self::write_biblatex_row(f, "pages", &self.pages)?;
        Self::write_biblatex_row(f, "year", &self.year)?;
        Self::write_biblatex_row(f, "author", &self.author)
    }
}
