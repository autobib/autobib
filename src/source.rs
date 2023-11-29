pub mod test;

pub use crate::record::{Entry, RecordError};

// TODO: improve this trait with more info on implementation
pub trait RecordSource {
    const SOURCE_NAME: &'static str;

    fn is_valid_id(&self, id: &str) -> bool;
    fn get_record(&self, id: &str) -> Result<Option<Entry>, RecordError>;
}
