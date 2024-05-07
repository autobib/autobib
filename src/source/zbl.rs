use regex::Regex;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_bibtex::de::Deserializer;

use crate::RecordError;
use crate::RecordId;

const ZBL_IDENTIFIER_REGEX: &'static str = r"^[0-9]{4}\.[0-9]{5}$";

pub fn is_valid_id(id: &str) -> bool {
    let zbl_identifier_regex = Regex::new(ZBL_IDENTIFIER_REGEX).unwrap();
    zbl_identifier_regex.is_match(id)
}

#[derive(Debug, Deserialize, PartialEq)]
struct OnlyEntryKey<'r> {
    entry_key: &'r str,
}

pub fn get_canonical(id: &str) -> Result<Option<RecordId>, RecordError> {
    let response = reqwest::blocking::get(format!("https://zbmath.org/bibtex/{id}.bib"))?;

    let body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(RecordError::UnexpectedStatusCode(code)),
    };

    let mut entry_iter = Deserializer::from_slice(&body).into_iter_entry();

    match entry_iter.next() {
        Some(Ok(OnlyEntryKey { entry_key })) => {
            const PREFIX: &'static str = "zbMATH";
            if entry_key.starts_with(PREFIX) {
                Ok(Some(RecordId::from_parts(
                    "zbmath",
                    &entry_key[PREFIX.len()..],
                )))
            } else {
                Err(RecordError::UnexpectedFailure(
                    "zbMATH bibtex record has unexpected citation key format!".to_string(),
                ))
            }
        }
        _ => Err(RecordError::UnexpectedFailure(
            "zbMATH bibtex record is invalid bibtex!".to_string(),
        )),
    }
}
