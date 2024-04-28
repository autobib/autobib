use regex::Regex;
use reqwest::StatusCode;
use serde_bibtex::SliceReader;

use crate::source::Entry;
use crate::RecordError;

const ZBMATH_IDENTIFIER_REGEX: &'static str = r"^[0-9]{8}$";

pub fn is_valid_id(id: &str) -> bool {
    let zbmath_identifier_regex = Regex::new(ZBMATH_IDENTIFIER_REGEX).unwrap();
    zbmath_identifier_regex.is_match(id)
}

pub fn get_record(id: &str) -> Result<Option<Entry>, RecordError> {
    let response = reqwest::blocking::get(format!("https://zbmath.org/bibtex/{id}.bib"))?;

    let body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(RecordError::UnexpectedStatusCode(code)),
    };

    let mut entry_iter = SliceReader::new(&body)
        .deserialize()
        .into_iter_entry::<Entry>();

    match entry_iter.next() {
        Some(Ok(entry)) => Ok(Some(entry)),
        _ => Err(RecordError::UnexpectedFailure(
            "Failed to parse ZBMath bibtex response!".to_string(),
        )),
    }
}
