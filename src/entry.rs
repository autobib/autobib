use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub key: String,
    pub contents: AnonymousEntry,
}

impl Display for Entry {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "@{}{{{}", self.contents.entry_type, self.key)?;
        write!(f, "{}", self.contents.fields)?;
        write!(f, "\n}}")
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnonymousEntry {
    pub entry_type: String,
    pub fields: Fields,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Fields {
    pub title: Option<String>,
    pub author: Option<String>,
}

impl Default for Fields {
    fn default() -> Self {
        Self {
            title: None,
            author: None,
        }
    }
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
        Self::write_biblatex_row(f, &"title", &self.title)?;
        Self::write_biblatex_row(f, &"author", &self.author)
    }
}
