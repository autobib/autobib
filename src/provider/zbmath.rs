use serde_bibtex::de::Deserializer;

use super::{
    BodyBytes, Client, ProviderBibtex, ProviderError, RecordData, StatusCode, ValidationOutcome,
};

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
    // It might be tempting to use the zbMATH REST API (https://api.zbmath.org/v1/).
    // However, sometimes this API endpoint will return incomplete data as a result of
    // licensing issues. On the other hand, the BibTeX record always works.
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

    let mut entry_iter =
        Deserializer::from_slice(&body).into_iter_regular_entry::<ProviderBibtex>();

    match entry_iter.next() {
        Some(Ok(entry)) => Ok(Some(entry.try_into()?)),
        _ => Err(ProviderError::Unexpected(
            "zbMATH BibTeX record is invalid!".into(),
        )),
    }
}
