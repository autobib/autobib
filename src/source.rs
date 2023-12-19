pub mod arxiv;
pub mod test;

use crate::entry::AnonymousEntry;
pub use crate::record::RecordError;

// TODO: improve this trait
pub trait RecordSource {
    fn is_valid_id(&self, id: &str) -> bool;
    fn get_record(&self, id: &str) -> Result<Option<AnonymousEntry>, RecordError>;
}
