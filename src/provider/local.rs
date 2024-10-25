use super::{HttpClient, ProviderError, RecordData};

#[inline]
pub fn is_valid_id(id: &str) -> bool {
    // Validity as a sub-id for the `local` provider is equivalent to validity as an alias.
    let id = id.trim();
    !id.is_empty() && !id.contains(':')
}

#[inline]
pub fn get_record(id: &str, _client: &HttpClient) -> Result<Option<RecordData>, ProviderError> {
    // WARNING: we must return an error here, or the record will get cached locally which will
    // result in strange errors!
    Err(ProviderError::UndefinedLocal(id.into()))
}
