use std::sync::LazyLock;

use memchr::{memchr, memchr2, memchr3};
use regex::Regex;
use serde_bibtex::token::is_entry_key;

use crate::RecordId;

/// Move forward until all comments and whitespace are consumed.
fn comment_and_ws(buffer: &[u8], mut pos: usize) -> usize {
    while pos < buffer.len() {
        match buffer[pos] {
            b'%' => {
                if let Some(skip) = memchr(b'\n', &buffer[pos..]) {
                    pos += skip + 1;
                }
            }
            s if s.is_ascii_whitespace() => pos += 1,
            _ => return pos,
        }
    }
    buffer.len()
}

/// Try to parse a macro `\<name>` where `<name>` is ascii alphabetic or starred.
fn ascii_macro(buffer: &[u8], mut pos: usize) -> (Option<&str>, usize) {
    // check the first char
    if buffer[pos] == b'\\' {
        pos += 1;
    } else {
        return (None, pos);
    }

    // take characters as long as they are ascii alphabetic or `*`
    let mut end = pos;
    while end < buffer.len() && (buffer[end].is_ascii_alphabetic() || buffer[end] == b'*') {
        end += 1;
    }

    // found: cast to string
    // SAFETY: chars are ascii alphabetic or *
    if pos < end {
        (
            Some(unsafe { std::str::from_utf8_unchecked(&buffer[pos..end]) }),
            end,
        )
    } else if end == buffer.len() {
        (None, pos)
    // skip a character to handle the `\\` case
    } else {
        (None, pos + 1)
    }
}

/// Skip an optional argument to a macro.
fn macro_opt_argument(buffer: &[u8], mut pos: usize) -> usize {
    if let Some(b'[') = buffer.get(pos) {
        pos += 1;
        loop {
            if let Some(offset) = memchr2(b']', b'%', &buffer[pos..]) {
                pos += offset;
                match &buffer[pos] {
                    b']' => break pos + 1,
                    _ => pos = comment_and_ws(buffer, pos),
                }
            } else {
                break pos;
            }
        }
    } else {
        pos
    }
}

/// Return the argument of a macro, skipping any optional arguments and pruning comments and some
/// whitespace.
fn macro_argument(buffer: &[u8], mut pos: usize) -> (Option<String>, usize) {
    pos = comment_and_ws(buffer, pos);
    pos = macro_opt_argument(buffer, pos);
    pos = comment_and_ws(buffer, pos);
    if let Some(b'{') = buffer.get(pos) {
        pos += 1;
        let mut start = pos;
        let mut contents: Vec<u8> = Vec::new();
        loop {
            if let Some(offset) = memchr3(b'{', b'}', b'%', &buffer[pos..]) {
                pos += offset;
                match &buffer[pos] {
                    b'{' => {
                        break (None, pos + 1);
                    }
                    b'}' => {
                        contents.extend(&buffer[start..pos]);
                        break (String::from_utf8(contents).ok(), pos + 1);
                    }
                    _ => {
                        contents.extend(&buffer[start..pos]);
                        pos = comment_and_ws(buffer, pos);
                        start = pos;
                    }
                }
            } else {
                break (None, pos);
            }
        }
    } else {
        (None, pos)
    }
}

/// Parse the citation contents and append new keys to `keys`.
fn parse_cite_contents<T: Extend<RecordId>>(contents: &str, container: &mut T) {
    container.extend(
        contents
            .split(',')
            .map(str::trim)
            .filter(|k| *k != "*" && is_entry_key(k))
            .map(Into::into),
    );
}

static CITATION_MACRO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^[a-zA-Z]?[a-z]*cite\*?$)|(^[Cc]ite[a-z]*\*?$)").unwrap());

/// Check if the macro name is an expected citation macro.
fn is_citation_macro_name(cmd: &str) -> bool {
    CITATION_MACRO_RE.is_match(cmd)
}

/// Get all citation keys in the buffer.
///
/// Citekeys essentially appear in the buffer in the form `\...cite{key1, key2}`, though there is a decent
/// amount of extra work required to properly handle comments and other subtleties.
pub fn get_citekeys<T: Extend<RecordId>>(buffer: &[u8], container: &mut T) {
    let mut pos: usize = 0;

    while let Some(next) = memchr2(b'%', b'\\', &buffer[pos..]) {
        pos += next;
        match buffer[pos] {
            b'\\' => {
                let (opt_cmd, next) = ascii_macro(buffer, pos);
                pos = next;
                if let Some(cmd) = opt_cmd
                    && is_citation_macro_name(cmd)
                {
                    let (opt_contents, next) = macro_argument(buffer, pos);
                    pos = next;
                    if let Some(contents) = opt_contents {
                        parse_cite_contents(&contents, container);
                    }
                }
            }
            _ => match memchr(b'\n', &buffer[pos..]) {
                Some(skip) => {
                    pos += skip + 1;
                }
                None => break,
            },
        }
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeSet;
    use std::iter::zip;

    use super::*;
    use crate::CitationKey;

    #[test]
    fn test_citation_macro() {
        assert!(is_citation_macro_name("Cite"));
        assert!(is_citation_macro_name("Cite*"));
        assert!(is_citation_macro_name("autocite"));
        assert!(is_citation_macro_name("Parencite"));
        assert!(is_citation_macro_name("Citetwo"));

        assert!(!is_citation_macro_name("citE"));
        assert!(!is_citation_macro_name("cit"));
        assert!(!is_citation_macro_name(" cite"));
        assert!(!is_citation_macro_name(""));
        assert!(!is_citation_macro_name("cite**"));
    }

    #[test]
    fn test_get_citekeys_tex() {
        let contents = r"
            An explanation can be found in \cite[§2]{ref2} (see also \cite{ref1,
            ref3}).
\autocite{contains space}.
\Cite{ref4}
            "
        .as_bytes();

        let mut container = BTreeSet::new();

        get_citekeys(contents, &mut container);

        let expected = ["ref1", "ref2", "ref3", "ref4"];
        assert_eq!(container.len(), expected.len());
        for (exp, rec) in zip(expected.iter(), container.iter()) {
            assert_eq!(*exp, rec.name());
        }
    }
}
