use lazy_static::lazy_static;
use regex::Regex;
use reqwest::StatusCode;
use serde_bibtex::de::Deserializer;

use super::{HttpClient, ProviderBibtex, ProviderError, RecordData};

lazy_static! {
    static ref ZBMATH_IDENTIFIER_RE: Regex = Regex::new(r"^[0-9]{8}$").unwrap();
}

pub fn is_valid_id(id: &str) -> bool {
    ZBMATH_IDENTIFIER_RE.is_match(id)
}

pub fn get_record(id: &str, client: &HttpClient) -> Result<Option<RecordData>, ProviderError> {
    // It might be tempting to use the zbMATH REST API (https://api.zbmath.org/v1/).
    // However, sometimes this API endpoint will return incomplete data as a result of
    // licensing issues. On the other hand, bibtex record always works.
    let response = client.get(format!("https://zbmath.org/bibtex/{id}.bib"))?;

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
            "zbMATH bibtex record is invalid!".into(),
        )),
    }
}
