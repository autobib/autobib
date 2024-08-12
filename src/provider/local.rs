use super::{HttpClient, ProviderError, RecordData};

pub fn is_valid_id(_id: &str) -> bool {
    true
}

pub fn get_record(id: &str, _client: &HttpClient) -> Result<Option<RecordData>, ProviderError> {
    Err(ProviderError::UndefinedLocal(id.into()))
}
