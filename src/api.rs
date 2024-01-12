pub use crate::database::{CacheResponse, RecordDatabase};
pub use crate::record::*;

use crate::source::{arxiv, test, zbmath, Resolver, Validator};

/// Map the `source` part of a [`RecordId`] to a [`CanonicalSource`].
fn lookup_resolver(record_id: &RecordId) -> Result<Resolver, RecordError> {
    match record_id.source() {
        "arxiv" => Ok(arxiv::get_record),
        "test" => Ok(test::get_record),
        "zbmath" => Ok(zbmath::get_record),
        _ => Err(RecordError::InvalidSource(record_id.clone())),
    }
}

/// Map the `source` part of a [`RecordId`] to a [`CanonicalSource`].
fn lookup_validator(record_id: &RecordId) -> Option<Validator> {
    match record_id.source() {
        "arxiv" => Some(arxiv::is_valid_id),
        "test" => Some(test::is_valid_id),
        "zbmath" => Some(zbmath::is_valid_id),
        _ => None,
    }
}

pub enum ValidationResult {
    InvalidSource(String),
    InvalidSubId(String),
    Ok,
}

pub fn validate_record_id(record_id: &RecordId) -> ValidationResult {
    match lookup_validator(&record_id) {
        Some(validator) => {
            if validator(record_id.sub_id()) {
                ValidationResult::Ok
            } else {
                ValidationResult::InvalidSubId(record_id.sub_id().to_string())
            }
        }
        None => ValidationResult::InvalidSource(record_id.source().to_string()),
    }
}

/// Get the [`Record`] associated with a [`RecordId`].
pub fn get_record(
    db: &mut RecordDatabase,
    record_id: &RecordId,
) -> Result<Option<Record>, RecordError> {
    match db.get_cached_data(record_id)? {
        CacheResponse::Found(cached_record) => Ok(Some(cached_record)),
        CacheResponse::FoundNull(_attempted) => Ok(None),
        CacheResponse::NotFound => {
            let resolver = lookup_resolver(&record_id)?;

            match resolver(record_id.sub_id()) {
                Ok(Some(entry)) => {
                    let record = Record::new(record_id.clone(), entry);
                    db.set_cached_data(&record)?;
                    Ok(Some(record))
                }
                Ok(None) => {
                    db.set_cached_null_record(record_id)?;
                    Ok(None)
                }
                Err(err) => Err(err),
            }
        }
    }
}
