use regex::Regex;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_bibtex::de::Deserializer;

use super::{RemoteId, SourceError};

const ZBL_IDENTIFIER_REGEX: &str = r"^[0-9]{4}\.[0-9]{5}$";

pub fn is_valid_id(id: &str) -> bool {
    let zbl_identifier_regex = Regex::new(ZBL_IDENTIFIER_REGEX).unwrap();
    zbl_identifier_regex.is_match(id)
}

#[derive(Debug, Deserialize, PartialEq)]
struct OnlyEntryKey<'r> {
    entry_key: &'r str,
}

pub fn get_canonical(id: &str) -> Result<Option<RemoteId>, SourceError> {
    let response = reqwest::blocking::get(format!("https://zbmath.org/bibtex/{id}.bib"))?;

    let body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(SourceError::UnexpectedStatusCode(code)),
    };

    let mut entry_iter = Deserializer::from_slice(&body).into_iter_entry();

    match entry_iter.next() {
        Some(Ok(OnlyEntryKey { entry_key })) => {
            if let Some(identifier) = entry_key.strip_prefix("zbMATH") {
                Ok(Some(RemoteId::from_parts("zbmath", identifier)))
            } else {
                Err(SourceError::Unexpected(
                    "zbMATH bibtex record has unexpected citation key format!".to_string(),
                ))
            }
        }
        _ => Err(SourceError::Unexpected(
            "zbMATH bibtex record is invalid bibtex!".into(),
        )),
    }
}
