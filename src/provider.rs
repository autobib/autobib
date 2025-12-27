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
use ureq::http::StatusCode;

// re-imports exposed to provider implementations
use crate::{
    MappedKey, RemoteId,
    entry::{EntryData, EntryType, MutableEntryData},
    error::{ProviderError, RecordDataError},
    http::{BodyBytes, Client},
};

/// A resolver, which converts a `sub_id` into [`MutableEntryData`].
type Resolver<C> = fn(&str, &C) -> Result<Option<MutableEntryData>, ProviderError>;

/// A referrer, which converts a `sub_id` into [`RemoteId`].
type Referrer<C> = fn(&str, &C) -> Result<Option<RemoteId>, ProviderError>;

/// A validator, which checks that a `sub_id` is valid.
type Validator = fn(&str) -> ValidationOutcome;

/// A provider, which is either a [`Resolver`] or a [`Referrer`].
enum Provider<C: Client> {
    Resolver(Resolver<C>),
    Referrer(Referrer<C>),
}

pub const REMOTE_PROVIDERS: [&str; 8] =
    ["arxiv", "doi", "isbn", "jfm", "mr", "ol", "zbmath", "zbl"];

/// Map the `provider` part of a [`RemoteId`] to a [`Resolver`] or [`Referrer`].
#[inline]
fn lookup_provider<C: Client>(provider: &str) -> Provider<C> {
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
        if b { Self::Valid } else { Self::Invalid }
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

pub enum RemoteIdCandidate {
    /// The optimal identifier found was canonical.
    OptimalCanonical(MappedKey),
    /// The optimal identifier found was a reference identifier. This also includes the optimal
    /// canonical identifier.
    OptimalReference(MappedKey, Option<MappedKey>),
    /// No identifier could be determined.
    None,
}

pub fn determine_key_from_data<F, D: EntryData>(
    data: &D,
    config: &crate::config::Config<F>,
) -> RemoteIdCandidate
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
{
    determine_remote_id_candidates(data, |id| config.score_id(id), None, None)
}

/// Determine candidates for valid remote identifiers from the provided bibtex data.
///
/// The closure `f` is a scoring function for the candidates.
///
/// - If a canonical identifier could be found and it received the highest score, it is returned alone.
/// - If a reference identifier had the highest score, the canonical identifier with the highest score (if any) is returned as well.
pub fn determine_remote_id_candidates<K: Ord, D: EntryData, F: FnMut(&RemoteId) -> K>(
    data: &D,
    mut score: F,
    candidate_canonical: Option<MappedKey>,
    candidate_reference: Option<MappedKey>,
) -> RemoteIdCandidate {
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

    /// A closure to compare the new result with the existing optimal, updating if it is better.
    #[inline]
    fn update_in_place<K: Ord, F: FnOnce(&RemoteId) -> K>(
        provider: &str,
        sub_id: &str,
        score: F,
        best_canonical: &mut Option<(MappedKey, K)>,
        best_reference: &mut Option<(MappedKey, K)>,
    ) {
        if let Ok(mapped) = MappedKey::mapped_from_parts(provider, sub_id) {
            let to_update = if is_canonical(provider) {
                best_canonical
            } else {
                best_reference
            };

            let new_k = score(&mapped.mapped);

            match to_update {
                Some((m, k)) => {
                    if new_k > *k {
                        *k = new_k;
                        *m = mapped;
                    }
                }
                None => {
                    *to_update = Some((mapped, new_k));
                }
            };
        }
    }

    let mut br = candidate_canonical.map(|c| {
        let s = score(&c.mapped);
        (c, s)
    });
    let mut bc = candidate_reference.map(|c| {
        let s = score(&c.mapped);
        (c, s)
    });
    // first determine candidates using provider-specific fields
    for (name, value) in data.fields() {
        if let Some(provider) = field_name_to_provider(name) {
            update_in_place(provider, value, &mut score, &mut bc, &mut br);
        }
    }

    // next, determine candidates using the `eprint` and `eprinttype` fields
    if let Some(provider) = data.get_field("eprinttype")
        && let Some(sub_id) = data.get_field("eprint")
    {
        update_in_place(provider, sub_id, &mut score, &mut bc, &mut br);
    }

    // special handling for arxiv
    if data
        .get_field("archiveprefix")
        .is_some_and(|val| val == "arXiv")
        && let Some(sub_id) = data.get_field("eprint")
    {
        update_in_place("arxiv", sub_id, &mut score, &mut bc, &mut br);
    }

    match (bc, br) {
        (Some((c, score_c)), Some((r, score_r))) => {
            if score_c >= score_r {
                RemoteIdCandidate::OptimalCanonical(c)
            } else {
                RemoteIdCandidate::OptimalReference(r, Some(c))
            }
        }
        (Some((c, _)), None) => RemoteIdCandidate::OptimalCanonical(c),
        (None, Some((r, _))) => RemoteIdCandidate::OptimalReference(r, None),
        (None, None) => RemoteIdCandidate::None,
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

/// Given a sub-id, determine valid [`RemoteId`]s with the given `sub_id` which are also valid.
pub fn suggest_valid_remote_identifiers<E, F>(sub_id: &str, mut cb: F) -> Result<(), E>
where
    F: FnMut(RemoteId) -> Result<(), E>,
{
    for provider in REMOTE_PROVIDERS {
        if let Ok(new) = RemoteId::from_parts(provider, sub_id) {
            cb(new)?;
        }
    }
    Ok(())
}

/// Check if the given string corresponds to a valid provider.
#[inline]
pub fn is_valid_provider(provider: &str) -> bool {
    lookup_validator(provider).is_some()
}

#[inline]
pub fn is_canonical(provider: &str) -> bool {
    // FIXME: this is implemented twice
    // lookup_validator(provider).is_some()
    //     && matches!(lookup_provider::<C>(provider), Provider::Resolver(_))
    match provider {
        "arxiv" | "doi" | "local" | "mr" | "ol" | "zbmath" => true,
        "isbn" | "jfm" | "zbl" => false,
        _ => unreachable!(
            "Invalid provider '{provider}: an invalid provider should have been caught by a call to `lookup_validator`'!"
        ),
    }
}

#[inline]
pub fn is_reference(provider: &str) -> bool {
    // FIXME: this is implemented twice
    // lookup_validator(provider).is_some()
    //     && matches!(lookup_provider::<C>(provider), Provider::Referrer(_))
    match provider {
        "arxiv" | "doi" | "local" | "mr" | "ol" | "zbmath" => false,
        "isbn" | "jfm" | "zbl" => true,
        _ => unreachable!(
            "Invalid provider '{provider}: an invalid provider should have been caught by a call to `lookup_validator`'!"
        ),
    }
}

/// The outcome of resolving a provider and making the remote call
pub enum RemoteResponse {
    /// The provider was a [`Resolver`] and returned [`MutableEntryData`].
    Data(MutableEntryData),
    /// The provider was a [`Referrer`] and returned a new [`RemoteId`].
    Reference(RemoteId),
    /// The provider returned `None`.
    Null,
}

/// Obtain the [`RemoteResponse`] by looking up the [`RemoteId`] using the provided `client`.
#[inline]
pub fn get_remote_response<C: Client>(
    client: &C,
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
/// This struct can be fallibly converted into a [`MutableEntryData`].
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

impl TryFrom<ProviderBibtex> for MutableEntryData {
    type Error = RecordDataError;

    fn try_from(value: ProviderBibtex) -> Result<Self, Self::Error> {
        let ProviderBibtex { entry_type, fields } = value;
        let mut record_data = Self::try_new(entry_type.to_lowercase())?;
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
