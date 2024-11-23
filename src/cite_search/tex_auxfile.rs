use std::{str::from_utf8, sync::LazyLock};

use regex::bytes::Regex;
use serde_bibtex::token::is_entry_key;

use crate::RecordId;

static AUX_CITE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\abx@aux@cite\{[0-9]*\}\{([^\}]+)\}").unwrap());

/// Get all citation keys in the buffer.
///
/// Citekeys essentially appear in the buffer as in the form `\abx@aux@cite{...}{key}` where `...`
/// is a sequence of digits (possibly empty).
pub fn get_citekeys<T: Extend<RecordId>>(buffer: &[u8], container: &mut T) {
    container.extend(
        AUX_CITE_RE
            .captures_iter(buffer)
            // SAFETY: the regex has a non-optional capture group
            .filter_map(|c| from_utf8(c.get(1).unwrap().as_bytes()).ok())
            .filter(|s| is_entry_key(s) && s != &"*")
            .map(RecordId::from),
    );
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_find_aux_citekeys() {
        let input = br#"
\abx@aux@cite{0}{a1}
\abx@aux@cite{}{a2}
\abx@aux@cite{} {a3}
\abx@aux@cite{0}{a4,a5}
\abx@aux@cite{} {a 6}
        "#;
        let mut vec: Vec<RecordId> = Vec::new();
        get_citekeys(input, &mut vec);
        assert!(vec.len() == 2);
        for s in ["a1", "a2"] {
            assert!(vec.contains(&RecordId::from(s)));
        }
        for s in ["a3", "a4", "a5", "a 6"] {
            assert!(!vec.contains(&RecordId::from(s)));
        }
    }
}
