//! # Abstractions over providers
//! This module implements remote resource resolution.
//!
//! The fundamental types are [`Resolver`], [`Referrer`], and [`Validator`], which abstract over
//! resource acquisition and resolution from a provider.
pub mod arxiv;
pub mod doi;
pub mod jfm;
pub mod local;
pub mod mr;
pub mod zbl;
pub mod zbmath;

use either::Either;
use serde::Deserialize;

// re-imports exposed to provider implementations
use crate::{
    db::RecordData,
    error::{ProviderError, RecordDataError},
    record::RemoteId,
    HttpClient,
};

/// A resolver, which converts a `sub_id` into [`RecordData`].
pub type Resolver = fn(&str, &HttpClient) -> Result<Option<RecordData>, ProviderError>;

/// A referrer, which converts a `sub_id` into [`RemoteId`].
pub type Referrer = fn(&str, &HttpClient) -> Result<Option<RemoteId>, ProviderError>;

/// A validator, which checks that a `sub_id` is valid.
pub type Validator = fn(&str) -> bool;

/// Map the `provider` part of a [`RemoteId`] to a [`Resolver`] or [`Referrer`].
pub(crate) fn lookup_provider(provider: &str) -> Either<Resolver, Referrer> {
    match provider {
        "arxiv" => Either::Left(arxiv::get_record),
        "doi" => Either::Left(doi::get_record),
        "jfm" => Either::Right(jfm::get_canonical),
        "local" => Either::Left(local::get_record),
        "mr" => Either::Left(mr::get_record),
        "zbmath" => Either::Left(zbmath::get_record),
        "zbl" => Either::Right(zbl::get_canonical),
        // SAFETY: An invalid provider should have been caught by a call to lookup_validator
        _ => panic!("Invalid provider '{provider}'!"),
    }
}

/// Validate a [`RemoteId`].
pub(crate) fn lookup_validator(provider: &str) -> Option<Validator> {
    match provider {
        "arxiv" => Some(arxiv::is_valid_id),
        "doi" => Some(doi::is_valid_id),
        "jfm" => Some(jfm::is_valid_id),
        "local" => Some(local::is_valid_id),
        "mr" => Some(mr::is_valid_id),
        "zbmath" => Some(zbmath::is_valid_id),
        "zbl" => Some(zbl::is_valid_id),
        _ => None,
    }
}

/// A receiving struct type useful for deserializing BibTeX from a provider.
///
/// This struct can be fallibly converted into a [`RecordData`].
#[derive(Debug, Deserialize)]
struct ProviderBibtex {
    entry_type: String,
    fields: ProviderBibtexFields,
}

/// The fields of a [`ProviderBibtex`] struct.
///
/// The aliases are required to handle <https://zbmath.org> BibTeX field name formatting.
/// This can be written in a more robust way if
/// <https://github.com/serde-rs/serde/pull/1902> or
/// <https://github.com/serde-rs/serde/pull/2161> are merged.
///
/// DO NOT USE [`serde_aux` case insensitive
/// deserialization](https://docs.rs/serde-aux/latest/serde_aux/container_attributes/fn.deserialize_struct_case_insensitive.html).
/// The problem is that `serde_aux` internally first deserializes to a map, and then deserializes
/// into a struct. Since `serde_bibtex` uses skipped fields to ignore undefined macros,
/// this can/will cause problems when deserializing.
#[derive(Debug, Default, Deserialize)]
struct ProviderBibtexFields {
    #[serde(alias = "Title", alias = "TITLE")]
    pub title: Option<String>,
    #[serde(alias = "Author", alias = "AUTHOR")]
    pub author: Option<String>,
    #[serde(alias = "Journal", alias = "JOURNAL")]
    pub journal: Option<String>,
    #[serde(alias = "Volume", alias = "VOLUME")]
    pub volume: Option<String>,
    #[serde(alias = "Pages", alias = "PAGES")]
    pub pages: Option<String>,
    #[serde(alias = "Year", alias = "YEAR")]
    pub year: Option<String>,
    #[serde(alias = "MRNUMBER")]
    pub mrnumber: Option<String>,
    #[serde(alias = "DOI")]
    pub doi: Option<String>,
    #[serde(alias = "Language", alias = "LANGUAGE")]
    pub language: Option<String>,
    #[serde(alias = "Zbl")]
    pub zbl: Option<String>,
    #[serde(alias = "zbMATH")]
    pub zbmath: Option<String>,
}

macro_rules! convert_field {
    ($fields:ident, $record_data:ident, $field:ident) => {
        if let Some($field) = $fields.$field {
            $record_data.try_insert(stringify!($field).into(), $field)?;
        };
    };
    ($fields:ident, $record_data:ident, $field:ident, $($tail:ident),+) => {
        convert_field!($fields, $record_data, $field);
        convert_field!($fields, $record_data, $($tail),+);
    };
}

impl TryFrom<ProviderBibtex> for RecordData {
    type Error = RecordDataError;

    fn try_from(value: ProviderBibtex) -> Result<Self, Self::Error> {
        let ProviderBibtex { entry_type, fields } = value;
        let mut record_data = RecordData::try_new(entry_type.to_lowercase())?;
        convert_field!(
            fields,
            record_data,
            title,
            author,
            journal,
            volume,
            pages,
            year,
            mrnumber,
            doi,
            language,
            zbl
        );

        // pad zeros for zbmath
        if let Some(field) = fields.zbmath {
            record_data.try_insert("zbmath".to_owned(), format!("{field:0>8}"))?;
        };

        Ok(record_data)
    }
}
