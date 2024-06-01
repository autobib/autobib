use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct KeyedEntry {
    pub key: String,
    pub contents: Entry,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub entry_type: String,
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

/// A container for bibtex fields.
///
/// The aliases are required to handle zbmath.org bibtex field name formatting.
/// This is a bit more robust if https://github.com/serde-rs/serde/pull/1902 or
/// https://github.com/serde-rs/serde/pull/2161 is merged...
///
/// DO NOT USE `serde_aux::container_attributes::deserialize_struct_case_insensitive`.
/// The problem is that `serde_aux` internally first deserializes to a map, and then deserializes
/// into a struct. Since `serde_bibtex` uses skipped fields to ignore undefined macros,
/// this can/will cause problems when deserializing.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Fields {
    #[serde(alias = "Title")]
    pub title: Option<String>,
    #[serde(alias = "Author")]
    pub author: Option<String>,
    #[serde(alias = "Journal")]
    pub journal: Option<String>,
    #[serde(alias = "Volume")]
    pub volume: Option<String>,
    #[serde(alias = "Pages")]
    pub pages: Option<String>,
    #[serde(alias = "Year")]
    pub year: Option<String>,
    #[serde(alias = "DOI")]
    pub doi: Option<String>,
    pub arxiv: Option<String>,
    #[serde(alias = "Language")]
    pub language: Option<String>,
}

// TODO: once serde_bibtex::serialize is implemented, this can be deleted

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

impl Display for Fields {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write_biblatex_row(f, "title", &self.title)?;
        write_biblatex_row(f, "author", &self.author)?;
        write_biblatex_row(f, "journal", &self.journal)?;
        write_biblatex_row(f, "volume", &self.volume)?;
        write_biblatex_row(f, "pages", &self.pages)?;
        write_biblatex_row(f, "year", &self.year)?;
        write_biblatex_row(f, "doi", &self.doi)?;
        write_biblatex_row(f, "arxiv", &self.arxiv)?;
        write_biblatex_row(f, "language", &self.language)?;
        Ok(())
    }
}
