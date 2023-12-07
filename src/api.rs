pub use crate::database::{CacheResponse, RecordDatabase};
pub use crate::record::*;
use crate::source::test::TestRecordSource;
use crate::source::RecordSource;

// TODO: this function does not actually work with more than one RecordSource...
/// Determine the record source corresponding to the name.
fn lookup_record_source(record_id: &RecordId) -> Result<impl RecordSource, RecordError> {
    match record_id.source() {
        "test" => Ok(TestRecordSource {}),
        _ => Err(RecordError::InvalidSource(record_id.clone())),
    }
}

/// Get the record associated with record_id
pub fn get_record(db: &mut RecordDatabase, record_id: RecordId) -> Result<Record, RecordError> {
    match db.get_cached_data(record_id)? {
        CacheResponse::Found(cached_record) => Ok(cached_record),
        CacheResponse::NotFound(record_id) => {
            let record_source = lookup_record_source(&record_id)?;

            if record_source.is_valid_id(record_id.sub_id()) {
                match record_source.get_record(record_id.sub_id()) {
                    Ok(Some(entry)) => {
                        let record = Record::new(record_id, Some(entry));
                        db.set_cached_data(&record)?;
                        Ok(record)
                    }
                    Ok(None) => Ok(Record::new(record_id, None)),
                    Err(err) => Err(err),
                }
            } else {
                Err(RecordError::InvalidSubId(record_id))
            }
        }
    }
}
