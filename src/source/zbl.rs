use reqwest::StatusCode;
use serde::Deserialize;
use serde_bibtex::SliceReader;

use crate::RecordError;
use crate::RecordId;

#[derive(Debug, Deserialize, PartialEq)]
struct OnlyCitationKey<'r> {
    citation_key: &'r str,
}

pub fn is_valid_id(_id: &str) -> bool {
    // TODO: fixme
    true
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
            if key.starts_with("zbMATH") {
                Ok(Some(RecordId::from_parts("zbmath", &key[6..])))
            } else {
                Err(RecordError::Incomplete) // TODO: fixme
            }
        }
        _ => Err(RecordError::Incomplete), // TODO: fixme
    }
}
