pub use crate::database::{CacheResponse, RecordDatabase};
pub use crate::record::*;

use crate::entry::Entry;
use crate::source::{arxiv, test, zbl, zbmath, Referrer, Resolver, Source, Validator};

/// Map the `source` part of a [`RecordId`] to a [`Source`].
fn lookup_source(record_id: &RecordId) -> Result<Source, RecordError> {
    match record_id.source() {
        "arxiv" => Ok(Source::Canonical(arxiv::get_record)),
        "test" => Ok(Source::Canonical(test::get_record)),
        "zbmath" => Ok(Source::Canonical(zbmath::get_record)),
        "zbl" => Ok(Source::Reference(zbmath::get_record, zbl::get_canonical)),
        _ => Err(RecordError::InvalidSource(record_id.clone())),
    }
}

/// Validate a [`RecordId`].
fn lookup_validator(record_id: &RecordId) -> Option<Validator> {
    match record_id.source() {
        "arxiv" => Some(arxiv::is_valid_id),
        "test" => Some(test::is_valid_id),
        "zbmath" => Some(zbmath::is_valid_id),
        "zbl" => Some(zbl::is_valid_id),
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

fn resolve_record_helper(
    resolver: Resolver,
    db: &mut RecordDatabase,
    record_id: &RecordId,
) -> Result<Option<Entry>, RecordError> {
    match resolver(record_id.sub_id()) {
        Ok(Some(entry)) => {
            db.set_cached_data(&record_id, &entry)?;
            Ok(Some(entry))
        }
        Ok(None) => {
            db.set_cached_null_record(&record_id)?;
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

/// Get the [`Record`] associated with a [`RecordId`].
pub fn get_record(
    db: &mut RecordDatabase,
    record_id: &RecordId,
) -> Result<Option<Entry>, RecordError> {
    match db.get_cached_data(record_id)? {
        CacheResponse::Found(cached_record) => Ok(Some(cached_record)),
        CacheResponse::FoundNull(_attempted) => Ok(None),
        CacheResponse::NotFound => {
            // Resolve the reference, if required...
            let (resolver, new_record_id) = match lookup_source(&record_id)? {
                Source::Canonical(resolver) => (resolver, record_id.clone()),
                Source::Reference(resolver, referrer) => {
                    match referrer(record_id.sub_id()) {
                        // TODO: cache here
                        Ok(Some(new_record_id)) => (resolver, new_record_id),
                        Ok(None) => todo!(),
                        Err(_) => todo!(),
                    }
                }
            };

            // ...then look up the record.
            resolve_record_helper(resolver, db, &new_record_id)
        }
    }
}
