use lazy_static::lazy_static;
use regex::Regex;
use reqwest::StatusCode;
use serde_bibtex::de::Deserializer;

use super::{Entry, SourceError};

lazy_static! {
    static ref DOI_IDENTIFIER_RE: Regex =
        Regex::new(r"^(10.\d{4,9}/[-._;()/:a-zA-Z0-9]+)|(10.1002/[^\s]+)$").unwrap();
}

pub fn is_valid_id(id: &str) -> bool {
    DOI_IDENTIFIER_RE.is_match(id)
}

pub fn get_record(id: &str) -> Result<Option<Entry>, SourceError> {
    let response = reqwest::blocking::get(format!(
        "https://api.crossref.org/works/{id}/transform/application/x-bibtex"
    ))?;

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
            "CrossRef bibtex record is invalid bibtex!".into(),
        )),
    }
}
