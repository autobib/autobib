pub use crate::database::{CacheResponse, RecordDatabase};
pub use crate::record::*;
use crate::source::RecordSource;
use crate::source::{arxiv::ArxivRecordSource, test::TestRecordSource, zbmath::ZBMathRecordSource};

// TODO: implement this statically?
fn lookup_record_source(record_id: &RecordId) -> Result<&'static dyn RecordSource, RecordError> {
    match record_id.source() {
        "arxiv" => Ok(&ArxivRecordSource {}),
        "test" => Ok(&TestRecordSource {}),
        "zbmath" => Ok(&ZBMathRecordSource {}),
        _ => Err(RecordError::InvalidSource(record_id.clone())),
    }
}

/// Get the record associated with record_id
pub fn get_record(
    db: &mut RecordDatabase,
    record_id: &RecordId,
) -> Result<Option<Record>, RecordError> {
    match db.get_cached_data(record_id)? {
        CacheResponse::Found(cached_record) => Ok(Some(cached_record)),
        CacheResponse::FoundNull(attempted) => Ok(None),
        CacheResponse::NotFound => {
            let record_source = lookup_record_source(&record_id)?;

            if record_source.is_valid_id(record_id.sub_id()) {
                match record_source.get_record(record_id.sub_id()) {
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
