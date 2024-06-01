pub mod arxiv;
pub mod doi;
pub mod jfm;
pub mod zbl;
pub mod zbmath;

use crate::entry::{Entry, Fields};
use crate::error::SourceError;
use crate::record::RemoteId;

use either::Either;

/// A resolver, which converts a `sub_id` into an [`Entry`].
pub type Resolver = fn(&str) -> Result<Option<Entry>, SourceError>;
/// A referrer, which converts a `sub_id` into a [`RemoteId`].
pub type Referrer = fn(&str) -> Result<Option<RemoteId>, SourceError>;
/// A validator, which checks that a `sub_id` is valid.
pub type Validator = fn(&str) -> bool;

/// Map the `source` part of a [`RemoteId`] to a [`Resolver`] or [`Referrer`].
pub fn lookup_source(source: &str) -> Either<Resolver, Referrer> {
    match source {
        "arxiv" => Either::Left(arxiv::get_record),
        "doi" => Either::Left(doi::get_record),
        "jfm" => Either::Right(jfm::get_canonical),
        "zbmath" => Either::Left(zbmath::get_record),
        "zbl" => Either::Right(zbl::get_canonical),
        // SAFETY: An invalid source should have been caught by a call to lookup_validator
        _ => panic!("Invalid source '{source}'!"),
    }
}

/// Validate a [`RemoteId`].
pub fn lookup_validator(source: &str) -> Option<Validator> {
    match source {
        "arxiv" => Some(arxiv::is_valid_id),
        "doi" => Some(doi::is_valid_id),
        "jfm" => Some(jfm::is_valid_id),
        "zbmath" => Some(zbmath::is_valid_id),
        "zbl" => Some(zbl::is_valid_id),
        _ => None,
    }
}
