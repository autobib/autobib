use super::{MutableEntryData, ProviderError, ValidationOutcome};

#[inline]
pub fn is_valid_id(id: &str) -> ValidationOutcome {
    // Validity as a sub-id for the `local` provider is equivalent to validity as an alias.
    (id.trim().len() == id.len() && !id.is_empty() && !id.contains(':')).into()
}

#[inline]
pub fn get_record<C>(id: &str, _client: &C) -> Result<Option<MutableEntryData>, ProviderError> {
    // WARNING: we must return an error here, or the record will get cached locally which will
    // result in strange errors!
    Err(ProviderError::UnexpectedLocal(id.into()))
}
