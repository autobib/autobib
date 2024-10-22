use super::{HttpClient, ProviderError, RecordData};

#[inline]
pub fn is_valid_id(_id: &str) -> bool {
    true
}

#[inline]
pub fn get_record(id: &str, _client: &HttpClient) -> Result<Option<RecordData>, ProviderError> {
    // WARNING: you must return an error here, or the record will get cached locally which will
    // result in strange errors!
    Err(ProviderError::UndefinedLocal(id.into()))
}
