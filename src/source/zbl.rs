use regex::Regex;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_bibtex::SliceReader;

use crate::RecordError;
use crate::RecordId;

const ZBL_IDENTIFIER_REGEX: &'static str = r"^[0-9]{4}\.[0-9]{5}$";

pub fn is_valid_id(id: &str) -> bool {
    let zbl_identifier_regex = Regex::new(ZBL_IDENTIFIER_REGEX).unwrap();
    zbl_identifier_regex.is_match(id)
}

#[derive(Debug, Deserialize, PartialEq)]
struct OnlyCitationKey<'r> {
    citation_key: &'r str,
}

pub fn get_canonical(id: &str) -> Result<Option<RecordId>, RecordError> {
    let response = reqwest::blocking::get(format!("https://zbmath.org/bibtex/{}.bib", id))?;

    let body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        _ => {
            return Err(RecordError::Incomplete);
        } //TODO: fixme
    };

    let mut entry_iter = SliceReader::new(&body).deserialize().into_iter_entry();

    match entry_iter.next() {
        Some(Ok(OnlyCitationKey { citation_key: key })) => {
            const PREFIX: &'static str = "zbMATH";
            if key.starts_with(PREFIX) {
                Ok(Some(RecordId::from_parts("zbmath", &key[PREFIX.len()..])))
            } else {
                Err(RecordError::Incomplete) // TODO: fixme
            }
        }
        _ => Err(RecordError::Incomplete), // TODO: fixme
    }
}
