use regex::Regex;
use reqwest::StatusCode;
use serde::Deserialize;

use super::{RemoteId, SourceError};

const JFM_IDENTIFIER_REGEX: &str = r"^[0-9]{2}\.[0-9]{4}\.[0-9]{2}$";

pub fn is_valid_id(id: &str) -> bool {
    let jfm_identifier_regex = Regex::new(JFM_IDENTIFIER_REGEX).unwrap();
    jfm_identifier_regex.is_match(id)
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
