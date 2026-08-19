//! # DOI provider
//!
//! It's hard to make a good rule for all DOIs. The one we have here has good enough coverage. It
//! works for all modern DOIs and most weird old DOIs.
//!
//! References:
//!
//! - [DOI Handbook](https://www.doi.org/doi-handbook/html/)
use std::sync::LazyLock;

use regex::Regex;
use serde_bibtex::de::Deserializer;

use super::{
    BodyBytes, Client, Ctx, MutableEntryData, ProviderBibtex, ProviderError, StatusCode,
    ValidationOutcome,
};

static DOI_IDENTIFIER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A(?:10\.\d{4,9}/[-._;()/:a-zA-Z0-9]+|10\.1002/[^\s]+)\z").unwrap()
});

pub fn is_valid_id(id: &str) -> ValidationOutcome {
    DOI_IDENTIFIER_RE.is_match(id).into()
}

pub fn get_record<C: Client>(
    id: &str,
    ctx: Ctx<C>,
) -> Result<Option<MutableEntryData>, ProviderError> {
    let response = ctx.client().get(format!(
        "https://api.crossref.org/works/{id}/transform/application/x-bibtex"
    ))?;

    let body = match response.status() {
        StatusCode::OK => response.into_body().bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(ProviderError::UnexpectedStatusCode(code)),
    };

    let mut entry_iter =
        Deserializer::from_slice(&body).into_iter_regular_entry::<ProviderBibtex>();

    match entry_iter.next() {
        Some(Ok(entry)) => Ok(Some(entry.try_into()?)),
        _ => Err(ProviderError::Unexpected(
            "CrossRef BibTeX record is invalid!".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid() {
        assert_eq!(
            is_valid_id("10.4007/annals.2014.180.2.7"),
            ValidationOutcome::Valid
        );
        assert_eq!(
            is_valid_id("10.1002/(SICI)1097-0312(199611)49:5<659::AID-CPA4>3.0.CO;2-L"),
            ValidationOutcome::Valid
        );
        assert_eq!(is_valid_id("10x1234/foo"), ValidationOutcome::Invalid);
        assert_eq!(is_valid_id("10.1234/foo!"), ValidationOutcome::Invalid);
    }
}
