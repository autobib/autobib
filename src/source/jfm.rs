use lazy_static::lazy_static;
use regex::Regex;
use reqwest::StatusCode;
use serde::Deserialize;

use super::{RemoteId, SourceError};

lazy_static! {
    static ref JFM_IDENTIFIER_RE: Regex = Regex::new(r"^[0-9]{2}\.[0-9]{4}\.[0-9]{2}$").unwrap();
}

pub fn is_valid_id(id: &str) -> bool {
    JFM_IDENTIFIER_RE.is_match(id)
}

#[derive(Debug, Deserialize, PartialEq)]
struct OnlyEntryKey<'r> {
    entry_key: &'r str,
}

pub fn get_canonical(id: &str) -> Result<Option<RemoteId>, SourceError> {
    let response = reqwest::blocking::get(format!("https://zbmath.org/bibtex/{id}.bib"))?;

    let _body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(SourceError::UnexpectedStatusCode(code)),
    };

    // unfortunately need to manually search through the XML response, perhaps with a regex
    Err(SourceError::Unexpected("Not implemented!".into()))
}
