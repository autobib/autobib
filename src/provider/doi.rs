use lazy_static::lazy_static;
use regex::Regex;
use reqwest::StatusCode;
use serde_bibtex::de::Deserializer;

use super::{HttpClient, ProviderBibtex, ProviderError, RecordData};

lazy_static! {
    static ref DOI_IDENTIFIER_RE: Regex =
        Regex::new(r"^(10.\d{4,9}/[-._;()/:a-zA-Z0-9]+)|(10.1002/[^\s]+)$").unwrap();
}

pub fn is_valid_id(id: &str) -> bool {
    DOI_IDENTIFIER_RE.is_match(id)
}

pub fn get_record(id: &str, client: &HttpClient) -> Result<Option<RecordData>, ProviderError> {
    let response = client.get(format!(
        "https://api.crossref.org/works/{id}/transform/application/x-bibtex"
    ))?;

    let body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(ProviderError::UnexpectedStatusCode(code)),
    };

    let mut entry_iter =
        Deserializer::from_slice(&body).into_iter_regular_entry::<ProviderBibtex>();

    match entry_iter.next() {
        Some(Ok(entry)) => Ok(Some(entry.try_into()?)),
        _ => Err(ProviderError::Unexpected(
            "CrossRef bibtex record is invalid bibtex!".into(),
        )),
    }
}
