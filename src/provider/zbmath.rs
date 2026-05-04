mod response;

use super::{BodyBytes, Client, MutableEntryData, ProviderError, StatusCode, ValidationOutcome};

use self::response::Response;

pub fn is_valid_id(id: &str) -> ValidationOutcome {
    // `10` is a rather arbitrary choice; do this to avoid O(n) check if the id is unreasonably long
    // all ids should have length <= 8
    if id.len() >= 10 || !id.as_bytes().iter().all(u8::is_ascii_digit) {
        return ValidationOutcome::Invalid;
    }

    let trimmed = id.trim_start_matches('0');
    if trimmed.len() != id.len() {
        ValidationOutcome::Normalize(trimmed.into())
    } else {
        ValidationOutcome::Valid
    }
}

pub fn get_record<C: Client>(
    id: &str,
    client: &C,
) -> Result<Option<MutableEntryData>, ProviderError> {
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
        Ok(response) => Ok(Some(response.result.try_into()?)),
        Err(err) => Err(ProviderError::UnexpectedResponseFormat(err.to_string())),
    }
}
