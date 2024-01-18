pub use crate::database::{CacheResponse, RecordDatabase};
pub use crate::record::*;

use crate::entry::Entry;
use crate::source::{arxiv, test, zbl, zbmath, Resolver, Source, Validator};

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

pub fn validate_citation_key(citation_key: &CitationKey) -> ValidationResult {
    match citation_key {
        CitationKey::RecordId(record_id) => validate_record_id(&record_id),
        CitationKey::Alias(_) => ValidationResult::Ok,
    }
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

/// Resolve the [`RecordId`] using the [`Resolver`] and insert the appropriate cache into the
/// database.
fn resolve_helper(
    resolver: Resolver,
    db: &mut RecordDatabase,
    record_id: &RecordId,
    reference_id: Option<&RecordId>,
) -> Result<Option<Entry>, RecordError> {
    match resolver(record_id.sub_id()) {
        Ok(Some(entry)) => {
            db.set_cached_data(&record_id, &entry, reference_id)?;
            Ok(Some(entry))
        }
        Ok(None) => {
            db.set_cached_null_record(&record_id)?;
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

/// Get the [`Entry`] associated with a [`CitationKey`].
pub fn get_record(
    db: &mut RecordDatabase,
    citation_key: &CitationKey,
) -> Result<Option<Entry>, RecordError> {
    match db.get_cached_data(citation_key)? {
        CacheResponse::Found(cached_entry, _modified) => Ok(Some(cached_entry)),
        CacheResponse::FoundNull(_attempted) => Ok(None),
        CacheResponse::NullAlias => Ok(None),
        CacheResponse::NotFound(record_id) => {
            match lookup_source(&record_id)? {
                // record_id is a canonical source, so there is no alias to be set
                Source::Canonical(resolver) => resolve_helper(resolver, db, &record_id, None),
                // record_id is a reference source, so we must set the alias
                Source::Reference(resolver, referrer) => match referrer(record_id.sub_id()) {
                    // resolved to a real record_id
                    Ok(Some(new_record_id)) => {
                        resolve_helper(resolver, db, &new_record_id, Some(record_id))
                    }
                    Ok(None) => {
                        db.set_cached_null_record(&record_id)?;
                        Ok(None)
                    }
                    Err(why) => Err(why),
                },
            }
        }
    }
}
