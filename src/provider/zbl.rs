use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;
use serde_bibtex::de::Deserializer;

use super::{BodyBytes, Client, ProviderError, RemoteId, StatusCode, ValidationOutcome};

static ZBL_IDENTIFIER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9]{4}\.[0-9]{5}$").unwrap());

pub fn is_valid_id(id: &str) -> ValidationOutcome {
    ZBL_IDENTIFIER_RE.is_match(id).into()
}

#[derive(Debug, Deserialize, PartialEq)]
struct OnlyEntryKey<'r> {
    entry_key: &'r str,
}

pub fn get_canonical<C: Client>(id: &str, client: &C) -> Result<Option<RemoteId>, ProviderError> {
    let response = client.get(format!("https://zbmath.org/bibtex/{id}.bib"))?;

    let body = match response.status() {
        StatusCode::OK => response.into_body().bytes()?,
        StatusCode::FORBIDDEN => {
            return Err(ProviderError::Unexpected(
                "zbMATH server is temporarily inaccessible; try again later.".into(),
            ));
        }
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(ProviderError::UnexpectedStatusCode(code)),
    };

    let mut entry_iter = Deserializer::from_slice(&body).into_iter_regular_entry();

    match entry_iter.next() {
        Some(Ok(OnlyEntryKey { entry_key })) => {
            if let Some(identifier) = entry_key.strip_prefix("zbMATH") {
                Ok(Some(RemoteId::from_parts("zbmath", identifier)?))
            } else {
                Err(ProviderError::Unexpected(
                    "zbMATH BibTeX record has unexpected citation key format!".to_string(),
                ))
            }
        }
        _ => Err(ProviderError::Unexpected(
            "zbMATH BibTeX record is invalid!".into(),
        )),
    }
}
