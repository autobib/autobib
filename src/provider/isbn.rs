use reqwest::StatusCode;
use serde::Deserialize;

use super::{HttpClient, ProviderError, RemoteId, ValidationOutcome};

/// Convert an ascii digit into the actual numerical value of the digit
fn ascii_digit_to_u8(b: u8) -> Option<u8> {
    if b.is_ascii_digit() {
        Some(b - b'0')
    } else {
        None
    }
}

/// Convert an ISBN10 checksum to the the corresponding ascii byte.
fn checksum_10_to_ascii(ck: u8) -> u8 {
    let ck = 11 - (ck % 11);
    if ck == 10 {
        b'X'
    } else {
        ck + b'0'
    }
}

/// Convert an ISBN13 checksum to the the corresponding ascii byte.
fn checksum_13_to_ascii(ck: u8) -> u8 {
    b'0' + 10 - (ck % 10)
}

/// Compute the ISBN13 checksum, assuming that the hyphens have already been removed and `id`
/// has length at least 12. Returns `None` if the id contains any bytes which are not ASCII digits.
fn isbn_13_checksum(id: &[u8]) -> Option<u8> {
    let mut checksum = 0;
    for chunk in id.chunks(2).take(6) {
        checksum += ascii_digit_to_u8(chunk[0])? + 3 * ascii_digit_to_u8(chunk[1])?;
    }
    Some(checksum_13_to_ascii(checksum))
}

/// Compute the ISBN10 and ISBN13 checksums simultaneously, assuming that the hyphens
/// have already been removed and `id` has length at least 9. Returns `None` if the id
/// contained any bytes which are not ASCII.
fn isbn_10_checksum(id: &[u8]) -> Option<(u8, u8)> {
    let mut checksum_10 = 0;
    let mut carry = 0;

    // to convert ISBN 10 -> ISBN 13, prepend '978', and recompute checksum
    // here we compute the the initial checksum value
    const ISBN_13_INITIAL_CHECKSUM: u8 = 9 + 3 * 7 + 8;
    let mut checksum_13 = ISBN_13_INITIAL_CHECKSUM;

    // extract the first step for parity reasons
    let new_digit = ascii_digit_to_u8(id[0])?;
    carry += new_digit;
    checksum_10 += carry;
    checksum_13 += 3 * new_digit;

    // iterate over the remaining pairs
    for idx in 0..4 {
        let b1 = ascii_digit_to_u8(id[2 * idx + 1])?;
        let b2 = ascii_digit_to_u8(id[2 * idx + 2])?;

        // update checksum 10
        carry += b1;
        checksum_10 += carry;
        carry += b2;
        checksum_10 += carry;

        // update checksum 13
        checksum_13 += b1 + 3 * b2;
    }

    // re-add the carry since we skipped the final digit
    checksum_10 += carry;

    Some((
        checksum_10_to_ascii(checksum_10),
        checksum_13_to_ascii(checksum_13),
    ))
}

fn validate_isbn_13_no_hyphen(id: &str) -> ValidationOutcome {
    if isbn_13_checksum(id.as_bytes()).is_some_and(|ck| ck == id.as_bytes()[12]) {
        ValidationOutcome::Valid
    } else {
        ValidationOutcome::Invalid
    }
}

fn validate_isbn_10_no_hyphen(id: &str) -> ValidationOutcome {
    match isbn_10_checksum(id.as_bytes()) {
        Some((ck_10, ck_13)) if ck_10 == id.as_bytes()[9] => {
            let mut normalized = String::with_capacity(13);
            normalized.push_str("978");
            normalized.push_str(&id[..9]);
            normalized.push(ck_13 as char);
            ValidationOutcome::Normalize(normalized)
        }
        _ => ValidationOutcome::Invalid,
    }
}

fn dehyphenate(id: &str) -> String {
    id.chars().filter(|ch| *ch != '-').collect()
}

// formats we handle:
// ISBN 10: 111994239X; final 'digit' is checksum (0-9 or X for '10')
// ISBN 13: 9781119942399
// ISBN 10 hyphenated: 0-596-52068-7
// ISBN 13 hyphenated:  978-0-596-52068-7
// ISBN 13 only initial hyphenated: 978-1119942399
//
// hyphens can be in many locations; we just check for the correct number of hyphens:
// - ISBN 10: 0 or 3 hyphens
// - ISBN 13: 0, 1, or 4 hyphens
// therefore we can branch on (length, hyphens), in order of priority
// (13, 0) => ISBN13, no filtering needed
// (10, 0) => ISBN10, no filtering needed
// (14, 1) => ISBN13 after filtering
// (17, 4) => ISBN13 after filtering
// (13, 3) => ISBN10 after filtering
pub fn is_valid_id(id: &str) -> ValidationOutcome {
    let num_hyphens = id.bytes().filter(|b| *b == b'-').count();

    match (id.len(), num_hyphens) {
        (13, 0) => validate_isbn_13_no_hyphen(id),
        (10, 0) => validate_isbn_10_no_hyphen(id),
        (14, 1) | (17, 4) => {
            let dehyphenated = dehyphenate(id);
            if matches!(
                validate_isbn_13_no_hyphen(&dehyphenated),
                ValidationOutcome::Valid
            ) {
                ValidationOutcome::Normalize(dehyphenated)
            } else {
                ValidationOutcome::Invalid
            }
        }
        (13, 3) => {
            let dehyphenated = dehyphenate(id);
            validate_isbn_10_no_hyphen(&dehyphenated)
        }
        _ => ValidationOutcome::Invalid,
    }
}

#[derive(Deserialize)]
struct OLKeyExtractor {
    key: String,
}

pub fn get_canonical(id: &str, client: &HttpClient) -> Result<Option<RemoteId>, ProviderError> {
    let response = client.get(format!("https://openlibrary.org/isbn/{id}.json"))?;

    let body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(ProviderError::UnexpectedStatusCode(code)),
    };

    let extractor: OLKeyExtractor = match serde_json::from_slice(&body) {
        Ok(ext) => ext,
        Err(err) => return Err(ProviderError::Unexpected(err.to_string())),
    };

    match extractor.key.strip_prefix("/books/OL") {
        Some(ol_id) => Ok(Some(RemoteId::from_parts("ol", ol_id)?)),
        None => Err(ProviderError::Unexpected(
            "Open Library JSON response is invalid!".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid() {
        assert_eq!(is_valid_id("9781119942399"), ValidationOutcome::Valid);
        assert_eq!(
            is_valid_id("111994239X"),
            ValidationOutcome::Normalize("9781119942399".to_owned())
        );
        assert_eq!(
            is_valid_id("978-0-596-52068-7"),
            ValidationOutcome::Normalize("9780596520687".to_owned())
        );
        assert_eq!(
            is_valid_id("3642651852"),
            ValidationOutcome::Normalize("9783642651854".to_owned())
        );
        assert_eq!(
            is_valid_id("3-642-65185-2"),
            ValidationOutcome::Normalize("9783642651854".to_owned())
        );
    }
}
