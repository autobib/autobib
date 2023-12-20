use bibparser::{BibEntry, Parser};
use regex::Regex;
use reqwest::StatusCode;
use std::str::FromStr;

use crate::entry::Fields;
use crate::source::{Entry, RecordSource};
use crate::RecordError;

const ZBMATH_IDENTIFIER_REGEX: &'static str = r"^[0-9]{8}$";

pub struct ZBMathRecordSource;

impl RecordSource for ZBMathRecordSource {
    fn is_valid_id(&self, id: &str) -> bool {
        let zbmath_identifier_regex = Regex::new(ZBMATH_IDENTIFIER_REGEX).unwrap();
        zbmath_identifier_regex.is_match(id)
    }

    fn get_record(&self, id: &str) -> Result<Option<Entry>, RecordError> {
        let response = reqwest::blocking::get(format!("https://zbmath.org/bibtex/{}.bib", id))?;

        let body: String = match response.status() {
            StatusCode::OK => response.text()?,
            StatusCode::NOT_FOUND => {
                return Ok(None);
            }
            _ => {
                return Err(RecordError::Incomplete);
            } //TODO: fixme
        };

        // TODO: panicking on parse failure
        let mut p = Parser::from_str(&body).unwrap();
        match p.iter().next() {
            Some(result) => {
                let BibEntry {
                    kind: entry_type,
                    fields,
                    id: _,
                } = result.unwrap();
                Ok(Some(Entry {
                    entry_type: entry_type.to_lowercase(),
                    fields: Fields {
                        title: fields.get("Title").cloned(),
                        author: fields.get("Author").cloned(),
                        ..Fields::default()
                    },
                }))
            }
            None => panic!("No matching entry!"),
        }
    }
}
