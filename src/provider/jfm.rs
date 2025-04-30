use std::sync::LazyLock;

use regex::{Regex, bytes::Regex as BytesRegex};
use reqwest::StatusCode;

use super::{HttpClient, ProviderError, RemoteId, ValidationOutcome};

static JFM_IDENTIFIER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9]{2}\.[0-9]{4}\.[0-9]{2}$").unwrap());
static BIBTEX_LINK_RE: LazyLock<BytesRegex> =
    LazyLock::new(|| BytesRegex::new(r"/bibtex/([0-9]{8})\.bib").unwrap());

pub fn is_valid_id(id: &str) -> ValidationOutcome {
    JFM_IDENTIFIER_RE.is_match(id).into()
}

pub fn get_canonical(id: &str, client: &HttpClient) -> Result<Option<RemoteId>, ProviderError> {
    let url = format!("https://zbmath.org/{id}");
    let response = client.get(&url)?;

    let body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(ProviderError::UnexpectedStatusCode(code)),
    };

    let mut identifiers = Vec::new();
    for (_, [sub_id]) in BIBTEX_LINK_RE.captures_iter(&body).map(|c| c.extract()) {
        // SAFETY: since BIBTEX_LINK_RE only matches on ASCII bytes the match is guaranteed to be
        // valid UTF-8
        identifiers.push(unsafe { std::str::from_utf8_unchecked(sub_id) });
    }

    match &identifiers[..] {
        [] => Ok(None),
        [identifier, ..] => Ok(Some(RemoteId::from_parts("zbmath", identifier)?)),
        // TODO: maybe do something better than just taking the first identifier
        //       e.g. jfm:60.0017.02 has multiple associated identifiers
        // _ => Err(ProviderError::Unexpected(format!(
        //     "Request to '{url}' returned multiple identifiers: {}",
        //     identifiers.join(", ")
        // ))),
    }
}
