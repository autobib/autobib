pub use crate::database::{CacheResponse, RecordDatabase};
pub use crate::record::*;

use crate::source::{arxiv, test, zbmath, CanonicalSource};

/// Map the `source` part of a [`RecordId`] to a [`CanonicalSource`].
fn lookup_record_source(record_id: &RecordId) -> Result<CanonicalSource, RecordError> {
    match record_id.source() {
        "arxiv" => Ok((arxiv::get_record, arxiv::is_valid_id)),
        "test" => Ok((test::get_record, test::is_valid_id)),
        "zbmath" => Ok((zbmath::get_record, zbmath::is_valid_id)),
        _ => Err(RecordError::InvalidSource(record_id.clone())),
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
            let (resolver, validator) = lookup_record_source(&record_id)?;

            if validator(record_id.sub_id()) {
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
            } else {
                Err(RecordError::InvalidSubId(record_id.clone()))
            }
        }
    }
}
