pub mod arxiv;
pub mod test;
pub mod zbmath;

use crate::entry::Entry;
pub use crate::record::RecordError;

pub type Resolver = fn(&str) -> Result<Option<Entry>, RecordError>;
pub type Validator = fn(&str) -> bool;
