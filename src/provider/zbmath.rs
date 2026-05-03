mod response;

use super::{BodyBytes, Client, MutableEntryData, ProviderError, StatusCode, ValidationOutcome};

use self::response::Response;

pub fn is_valid_id(id: &str) -> ValidationOutcome {
    if id.len() == 8 && id.as_bytes().iter().all(u8::is_ascii_digit) {
        ValidationOutcome::Valid
    } else if id.len() <= 7 && id.as_bytes().iter().all(u8::is_ascii_digit) {
        // the `id.is_empty()` case is handled globally
        ValidationOutcome::Normalize(format!("{id:0>8}"))
    } else {
        ValidationOutcome::Invalid
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
