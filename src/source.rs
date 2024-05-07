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

pub enum SourceHandler {
    Canonical(Resolver),
    Reference(Resolver, Referrer),
}

/// Map the `source` part of a [`RecordId`] to a [`Source`].
pub fn lookup_source(source: &str) -> Result<SourceHandler, RecordError> {
    match source {
        "arxiv" => Ok(SourceHandler::Canonical(arxiv::get_record)),
        "test" => Ok(SourceHandler::Canonical(test::get_record)),
        "zbmath" => Ok(SourceHandler::Canonical(zbmath::get_record)),
        "zbl" => Ok(SourceHandler::Reference(
            zbmath::get_record,
            zbl::get_canonical,
        )),
        // SAFETY: An invalid source should have been caught by a call to lookup_validator
        _ => panic!("Invalid source!"),
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
