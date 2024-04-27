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

/// Map the `source` part of a [`RecordId`] to a [`Source`].
pub fn lookup_source(source: &str) -> Result<Source, RecordError> {
    match source {
        "arxiv" => Ok(Source::Canonical(arxiv::get_record)),
        "test" => Ok(Source::Canonical(test::get_record)),
        "zbmath" => Ok(Source::Canonical(zbmath::get_record)),
        "zbl" => Ok(Source::Reference(zbmath::get_record, zbl::get_canonical)),
        _ => Err(RecordError::InvalidSource(source.to_string())),
    }
}

/// Validate a [`RecordId`].
pub fn lookup_validator(source: &str) -> Option<Validator> {
    match source {
        "arxiv" => Some(arxiv::is_valid_id),
        "test" => Some(test::is_valid_id),
        "zbmath" => Some(zbmath::is_valid_id),
        "zbl" => Some(zbl::is_valid_id),
        _ => None,
    }
}
