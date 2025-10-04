mod response;

use super::{BodyBytes, Client, ProviderError, RecordData, StatusCode, ValidationOutcome};

use self::response::Response;

pub fn is_valid_id(id: &str) -> ValidationOutcome {
    if id.len() == 8 && id.as_bytes().iter().all(u8::is_ascii_digit) {
        ValidationOutcome::Valid
    } else if id.len() == 7 && id.as_bytes().iter().all(u8::is_ascii_digit) {
        let mut normalized = String::with_capacity(8);
        normalized.push('0');
        normalized.push_str(id);
        ValidationOutcome::Normalize(normalized)
    } else {
        ValidationOutcome::Invalid
    }
}

pub fn get_record<C: Client>(id: &str, client: &C) -> Result<Option<RecordData>, ProviderError> {
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
