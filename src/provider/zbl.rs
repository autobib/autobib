use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

use super::{BodyBytes, Client, ProviderError, RemoteId, StatusCode, ValidationOutcome};

#[derive(Deserialize)]
pub struct Response {
    pub result: EntryIdOnly,
}

#[derive(Deserialize)]
pub struct EntryIdOnly {
    id: u32,
}

static ZBL_IDENTIFIER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9]{4}\.[0-9]{5}$").unwrap());

pub fn is_valid_id(id: &str) -> ValidationOutcome {
    ZBL_IDENTIFIER_RE.is_match(id).into()
}

pub fn get_canonical<C: Client>(id: &str, client: &C) -> Result<Option<RemoteId>, ProviderError> {
    let response = client.get(format!("https://api.zbmath.org/v1/document/{id}"))?;

    let mut body = match response.status() {
        StatusCode::OK => response.into_body(),
        StatusCode::FORBIDDEN => {
            return Err(ProviderError::TemporaryFailure);
        }
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(ProviderError::UnexpectedStatusCode(code)),
    };

    match body.read_json::<Response>() {
        Ok(response) => Ok(Some(RemoteId::from_parts(
            "zbmath",
            &response.result.id.to_string(),
        )?)),
        Err(err) => Err(ProviderError::UnexpectedResponseFormat(err.to_string())),
    }
}
