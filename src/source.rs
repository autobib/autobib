pub mod arxiv;
pub mod test;
pub mod zbmath;

use crate::entry::Entry;
pub use crate::record::RecordError;

/// A pair `(resolver, validator)` where `resolver` is used to obtain a new record and `validator`
/// is used to perform inexpensive `sub_id` validation.
pub type CanonicalSource = (
    fn(&str) -> Result<Option<Entry>, RecordError>,
    fn(&str) -> bool,
);
