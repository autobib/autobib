use lazy_static::lazy_static;
use regex::{bytes::Regex as BytesRegex, Regex};
use reqwest::StatusCode;
use serde::Deserialize;

use super::{HttpClient, RemoteId, SourceError};

lazy_static! {
    static ref JFM_IDENTIFIER_RE: Regex = Regex::new(r"^[0-9]{2}\.[0-9]{4}\.[0-9]{2}$").unwrap();
    static ref BIBTEX_LINK_RE: BytesRegex = BytesRegex::new(r"/bibtex/([0-9]{8})\.bib").unwrap();
}

pub fn is_valid_id(id: &str) -> bool {
    JFM_IDENTIFIER_RE.is_match(id)
}

#[derive(Debug, Deserialize, PartialEq)]
struct OnlyEntryKey<'r> {
    entry_key: &'r str,
}

pub fn get_canonical(id: &str, client: &HttpClient) -> Result<Option<RemoteId>, SourceError> {
    let url = format!("https://zbmath.org/{id}");
    let response = client.get(&url)?;

    let body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(SourceError::UnexpectedStatusCode(code)),
    };

    let mut identifiers = Vec::new();
    for (_, [sub_id]) in BIBTEX_LINK_RE.captures_iter(&body).map(|c| c.extract()) {
        // SAFETY: since BIBTEX_LINK_RE only matches on ASCII bytes the match is guaranteed to be
        // valid UTF-8
        identifiers.push(unsafe { std::str::from_utf8_unchecked(sub_id) });
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
