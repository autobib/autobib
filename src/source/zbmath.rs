use regex::Regex;
use reqwest::StatusCode;
use serde_bibtex::de::Deserializer;

use super::{Entry, SourceError};

const ZBMATH_IDENTIFIER_REGEX: &str = r"^[0-9]{8}$";

pub fn is_valid_id(id: &str) -> bool {
    let zbmath_identifier_regex = Regex::new(ZBMATH_IDENTIFIER_REGEX).unwrap();
    zbmath_identifier_regex.is_match(id)
}

pub fn get_record(id: &str) -> Result<Option<Entry>, SourceError> {
    let response = reqwest::blocking::get(format!("https://zbmath.org/bibtex/{id}.bib"))?;

    let body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(SourceError::UnexpectedStatusCode(code)),
    };

    let mut entry_iter = Deserializer::from_slice(&body).into_iter_entry::<Entry>();

    match entry_iter.next() {
        Some(Ok(entry)) => Ok(Some(entry)),
        _ => Err(SourceError::Unexpected(
            "zbMATH bibtex record is invalid bibtex!".into(),
        )),
    }
}
