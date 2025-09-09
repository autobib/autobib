use serde::Deserialize;
use serde_bibtex::de::Deserializer;

use super::{
    Client, ProviderBibtex, ProviderError, RecordData, Response, StatusCode, ValidationOutcome,
};

#[allow(dead_code)]
#[derive(Deserialize)]
struct MathscinetRecord {
    bib: String,
    #[serde(rename = "paperId")]
    id: u32,
}

pub fn is_valid_id(id: &str) -> ValidationOutcome {
    (id.len() == 7 && id.as_bytes().iter().all(u8::is_ascii_digit)).into()
}

pub fn get_record<C: Client>(id: &str, client: &C) -> Result<Option<RecordData>, ProviderError> {
    let mut response = client.get(format!(
        "https://mathscinet.ams.org/mathscinet/api/publications/format?formats=bib&ids={id}"
    ))?;

    let body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(ProviderError::UnexpectedStatusCode(code)),
    };

    let (msc_record,): (MathscinetRecord,) = match serde_json::from_slice(&body) {
        Ok(record) => record,
        Err(err) => return Err(ProviderError::Unexpected(err.to_string())),
    };

    let mut entry_iter =
        Deserializer::from_str(&msc_record.bib).into_iter_regular_entry::<ProviderBibtex>();

    match entry_iter.next() {
        Some(Ok(entry)) => Ok(Some(entry.try_into()?)),
        _ => Err(ProviderError::Unexpected(
            "MathSciNet BibTeX record is invalid!".into(),
        )),
    }
}
