//! # Abstractions over providers
//! This module implements remote resource resolution.
//!
//! The fundamental types are [`Resolver`], [`Referrer`], and [`Validator`], which abstract over
//! resource acquisition and resolution from a provider.
mod arxiv;
mod doi;
mod isbn;
mod jfm;
mod local;
mod mr;
mod ol;
mod zbl;
mod zbmath;

use serde::Deserialize;

// re-imports exposed to provider implementations
use crate::{
    HttpClient, MappedKey, RemoteId,
    entry::{EntryData, EntryType, RecordData},
    error::{ProviderError, RecordDataError},
};

/// A resolver, which converts a `sub_id` into [`RecordData`].
type Resolver = fn(&str, &HttpClient) -> Result<Option<RecordData>, ProviderError>;

/// A referrer, which converts a `sub_id` into [`RemoteId`].
type Referrer = fn(&str, &HttpClient) -> Result<Option<RemoteId>, ProviderError>;

/// A validator, which checks that a `sub_id` is valid.
type Validator = fn(&str) -> ValidationOutcome;

/// A provider, which is either a [`Resolver`] or a [`Referrer`].
enum Provider {
    Resolver(Resolver),
    Referrer(Referrer),
}

/// Map the `provider` part of a [`RemoteId`] to a [`Resolver`] or [`Referrer`].
#[inline]
fn lookup_provider(provider: &str) -> Provider {
    match provider {
        "arxiv" => Provider::Resolver(arxiv::get_record),
        "doi" => Provider::Resolver(doi::get_record),
        "isbn" => Provider::Referrer(isbn::get_canonical),
        "jfm" => Provider::Referrer(jfm::get_canonical),
        "local" => Provider::Resolver(local::get_record),
        "mr" => Provider::Resolver(mr::get_record),
        "ol" => Provider::Resolver(ol::get_record),
        "zbmath" => Provider::Resolver(zbmath::get_record),
        "zbl" => Provider::Referrer(zbl::get_canonical),
        _ => unreachable!(
            "Invalid provider '{provider}: an invalid provider should have been caught by a call to `lookup_validator`'!"
        ),
    }
}

/// Validate a [`RemoteId`].
#[inline]
fn lookup_validator(provider: &str) -> Option<Validator> {
    match provider {
        "arxiv" => Some(arxiv::is_valid_id),
        "doi" => Some(doi::is_valid_id),
        "isbn" => Some(isbn::is_valid_id),
        "jfm" => Some(jfm::is_valid_id),
        "local" => Some(local::is_valid_id),
        "mr" => Some(mr::is_valid_id),
        "ol" => Some(ol::is_valid_id),
        "zbmath" => Some(zbmath::is_valid_id),
        "zbl" => Some(zbl::is_valid_id),
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
pub enum ValidationOutcome {
    Valid,
    Normalize(String),
    Invalid,
}

impl From<bool> for ValidationOutcome {
    fn from(b: bool) -> Self {
        if b {
            ValidationOutcome::Valid
        } else {
            ValidationOutcome::Invalid
        }
    }
}

/// The outcome of checking that a provider and sub_id are valid.
pub enum ValidationOutcomeExtended {
    /// The provider and sub_id are both valid.
    Valid,
    /// The provider is valid but the sub_id requires normalization.
    Normalize(String),
    /// The provider is invalid.
    InvalidProvider,
    /// The sub_id is invalid for the given provider.
    InvalidSubId,
}

/// Convert a bibtex field name whose value may contain an identifier for the returned provider.
#[inline]
fn field_name_to_provider(field_name: &str) -> Option<&'static str> {
    match field_name {
        "arxiv" => Some("arxiv"),
        "doi" => Some("doi"),
        "mrnumber" => Some("mr"),
        "zbl" => Some("zbl"),
        "zbmath" => Some("zbmath"),
        _ => None,
    }
}

/// Given a provider and sub-id, push to the provided buffer if the `provider` is accepted by
/// `preprocess` and the `provider:sub_id` is valid.
#[inline]
fn push_remote_id_if_valid<T, F: FnOnce(&str) -> Option<T>, G: FnOnce(MappedKey, T)>(
    provider: &str,
    sub_id: &str,
    preprocess: F,
    push: G,
) {
    if let Some(filtered) = preprocess(provider)
        && let Ok(remote_id) = MappedKey::mapped_from_parts(provider, sub_id)
    {
        push(remote_id, filtered);
    }
}

/// Determine candidates for valid remote identifiers from the provided bibtex data. Each
/// provider is passed to `preprocess`, and is only processed if `preprocess` returns `Some(T)`. The value
/// `T`, if `Some`, and the resulting [`MappedKey`], is subsequently passed to `push`.
pub fn determine_remote_id_candidates<
    T,
    D: EntryData,
    F: FnMut(&str) -> Option<T>,
    G: FnMut(MappedKey, T),
>(
    data: &D,
    mut preprocess: F,
    mut push: G,
) {
    // first determine candidates using provider-specific fields
    for (name, value) in data.fields() {
        if let Some(provider) = field_name_to_provider(name) {
            push_remote_id_if_valid(provider, value, &mut preprocess, &mut push);
        }
    }

    // next, determine candidates using the `eprint` and `eprinttype` fields
    if let Some(provider) = data.get_field("eprinttype")
        && let Some(sub_id) = data.get_field("eprint")
    {
        push_remote_id_if_valid(provider, sub_id, &mut preprocess, &mut push);
    }
}

/// Check that a given provider and sub_id are valid.
#[inline]
pub fn validate_provider_sub_id(provider: &str, sub_id: &str) -> ValidationOutcomeExtended {
    match lookup_validator(provider) {
        Some(validator) => match validator(sub_id) {
            ValidationOutcome::Valid => ValidationOutcomeExtended::Valid,
            ValidationOutcome::Normalize(s) => ValidationOutcomeExtended::Normalize(s),
            ValidationOutcome::Invalid => ValidationOutcomeExtended::InvalidSubId,
        },
        None => ValidationOutcomeExtended::InvalidProvider,
    }
}

/// Check if the given string corresponds to a valid provider.
#[inline]
pub fn is_valid_provider(provider: &str) -> bool {
    lookup_validator(provider).is_some()
}

#[inline]
pub fn is_canonical(provider: &str) -> bool {
    lookup_validator(provider).is_some()
        && matches!(lookup_provider(provider), Provider::Resolver(_))
}

#[inline]
pub fn is_reference(provider: &str) -> bool {
    lookup_validator(provider).is_some()
        && matches!(lookup_provider(provider), Provider::Referrer(_))
}

/// The outcome of resolving a provider and making the remote call
pub enum RemoteResponse {
    /// The provider was a [`Resolver`] and returned [`RecordData`].
    Data(RecordData),
    /// The provider was a [`Referrer`] and returned a new [`RemoteId`].
    Reference(RemoteId),
    /// The provider returned `None`.
    Null,
}

/// Obtain the [`RemoteResponse`] by looking up the [`RemoteId`] using the provided `client`.
#[inline]
pub fn get_remote_response(
    client: &HttpClient,
    remote_id: &RemoteId,
) -> Result<RemoteResponse, ProviderError> {
    match lookup_provider(remote_id.provider()) {
        Provider::Resolver(resolver) => match resolver(remote_id.sub_id(), client)? {
            Some(data) => Ok(RemoteResponse::Data(data)),
            None => Ok(RemoteResponse::Null),
        },
        Provider::Referrer(referrer) => match referrer(remote_id.sub_id(), client)? {
            Some(new_remote_id) => Ok(RemoteResponse::Reference(new_remote_id)),
            None => Ok(RemoteResponse::Null),
        },
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
    #[serde(alias = "Author", alias = "AUTHOR")]
    pub author: Option<String>,
    #[serde(alias = "DOI")]
    pub doi: Option<String>,
    #[serde(alias = "Editor")]
    pub editor: Option<String>,
    #[serde(alias = "Journal", alias = "JOURNAL")]
    pub journal: Option<String>,
    #[serde(alias = "Language", alias = "LANGUAGE")]
    pub language: Option<String>,
    #[serde(alias = "MRNUMBER")]
    pub mrnumber: Option<String>,
    #[serde(alias = "Pages", alias = "PAGES")]
    pub pages: Option<String>,
    #[serde(alias = "Publisher", alias = "PUBLISHER")]
    pub publisher: Option<String>,
    #[serde(alias = "Series", alias = "SERIES")]
    pub series: Option<String>,
    #[serde(alias = "Title", alias = "TITLE")]
    pub title: Option<String>,
    #[serde(alias = "Volume", alias = "VOLUME")]
    pub volume: Option<String>,
    #[serde(alias = "Year", alias = "YEAR")]
    pub year: Option<String>,
    #[serde(alias = "Zbl")]
    pub zbl: Option<String>,
    #[serde(alias = "zbMATH")]
    pub zbmath: Option<String>,
}

macro_rules! convert_field {
    ($fields:ident, $record_data:ident, $field:ident) => {
        if let Some($field) = $fields.$field {
            $record_data.check_and_insert(stringify!($field).into(), $field)?;
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
            author,
            editor,
            doi,
            journal,
            language,
            mrnumber,
            pages,
            publisher,
            series,
            title,
            volume,
            year,
            zbl
        );

        // pad zeros for zbmath
        if let Some(field) = fields.zbmath {
            record_data.check_and_insert("zbmath".to_owned(), format!("{field:0>8}"))?;
        };

        Ok(record_data)
    }
}
