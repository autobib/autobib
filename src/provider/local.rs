use super::{HttpClient, ProviderError, RecordData};

pub fn is_valid_id(_id: &str) -> bool {
    true
}

pub fn get_record(_id: &str, _client: &HttpClient) -> Result<Option<RecordData>, ProviderError> {
    Ok(None)
}
