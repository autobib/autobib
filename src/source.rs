pub mod arxiv;
pub mod test;
pub mod zbl;
pub mod zbmath;

use crate::entry::Entry;
use crate::record::RecordError;
use crate::RecordId;

pub type Resolver = fn(&str) -> Result<Option<Entry>, RecordError>;
pub type Referrer = fn(&str) -> Result<Option<RecordId>, RecordError>;
pub type Validator = fn(&str) -> bool;

pub enum Source {
    Canonical(Resolver),
    Reference(Resolver, Referrer),
}
