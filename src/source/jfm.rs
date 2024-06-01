use lazy_static::lazy_static;
use regex::Regex;
use reqwest::StatusCode;
use serde::Deserialize;

use super::{RemoteId, SourceError};

lazy_static! {
    static ref JFM_IDENTIFIER_RE: Regex = Regex::new(r"^[0-9]{2}\.[0-9]{4}\.[0-9]{2}$").unwrap();
    static ref BIBTEX_LINK_RE: Regex = Regex::new(r"/bibtex/([0-9]{8})\.bib").unwrap();
}

pub fn is_valid_id(id: &str) -> bool {
    JFM_IDENTIFIER_RE.is_match(id)
}

#[derive(Debug, Deserialize, PartialEq)]
struct OnlyEntryKey<'r> {
    entry_key: &'r str,
}

pub fn get_canonical(id: &str) -> Result<Option<RemoteId>, SourceError> {
    let url = format!("https://zbmath.org/{id}");
    let response = reqwest::blocking::get(&url)?;

    let body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(SourceError::UnexpectedStatusCode(code)),
    };

    let body_str = std::str::from_utf8(&body).map_err(|_| {
        SourceError::Unexpected(format!("Request to '{url}' returned invalid UTF-8."))
    })?;

    let mut identifiers = Vec::new();
    for (_, [sub_id]) in BIBTEX_LINK_RE.captures_iter(body_str).map(|c| c.extract()) {
        identifiers.push(sub_id);
    }

    match &identifiers[..] {
        [] => Ok(None),
        [identifier] => Ok(Some(RemoteId::from_parts("zbmath", identifier))),
        _ => Err(SourceError::Unexpected(format!(
            "Request to '{url}' returned multiple identifiers: {}",
            identifiers.join(", ")
        ))),
    }
}
