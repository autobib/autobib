pub mod arxiv;
pub mod test;
pub mod zbmath;

use crate::entry::Entry;
pub use crate::record::RecordError;

// TODO: improve this trait
pub trait RecordSource {
    fn is_valid_id(&self, id: &str) -> bool;
    fn get_record(&self, id: &str) -> Result<Option<Entry>, RecordError>;
}
