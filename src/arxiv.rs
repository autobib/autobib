use crate::{Record, RecordError, ValidationResult};
use std::error::Error;

pub fn validate_id(id: &str) -> ValidationResult {
    ValidationResult::Valid
}

pub fn get_record(id: &str) -> Result<Record, RecordError> {
    // Assume `id` is valid
    Ok(Record {
        repository: String::from("arxiv"),
        id: String::from(id),
        title: String::from("Autobib"),
        authors: vec![String::from("Alex Rutar"), String::from("Peiran Wu")],
    })
}
