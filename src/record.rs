mod key;

use std::fmt;

pub use key::*;

use crate::database::{CacheResponse, RecordDatabase};
use crate::entry::Entry;
use crate::source::{lookup_source, Resolver, Source};

#[derive(Debug)]
pub enum RecordError {
    InvalidSource(String),
    NetworkFailure(reqwest::Error),
    DatabaseFailure(rusqlite::Error),
    Incomplete,
}

impl From<rusqlite::Error> for RecordError {
    fn from(err: rusqlite::Error) -> Self {
        RecordError::DatabaseFailure(err)
    }
}

impl From<reqwest::Error> for RecordError {
    fn from(err: reqwest::Error) -> Self {
        RecordError::NetworkFailure(err)
    }
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordError::InvalidSource(source) => {
                write!(f, "'{}' is not a valid source.", source)
            }
            RecordError::DatabaseFailure(error) => write!(f, "Database failure: {}", error),
            RecordError::NetworkFailure(error) => write!(f, "Network failure: {}", error),
            RecordError::Incomplete => write!(f, "Incomplete record"),
        }
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
            match lookup_source(&record_id.source())? {
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
