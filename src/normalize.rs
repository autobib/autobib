//! Utilities for normalizing BibTeX data
use std::{slice::Iter, str::CharIndices};

use serde::Deserialize;

/// A normalization which can be applied to bibliographic record data.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Normalization {
    #[serde(default)]
    normalize_whitespace: bool,
    #[serde(default)]
    set_eprint: Vec<String>,
    #[serde(default)]
    strip_journal_series: bool,
}

/// Types which can be normalized by the operations specified in a [`Normalization`].
///
/// In practice, this is only implemented for [`RecordData`](crate::db::RecordData).
pub trait Normalize {
    /// Attempt to set the `eprint` and `eprinttype` BibTeX fields using the value of a provided
    /// BibTeX field from the `keys` iterator.
    ///
    /// Note that, if successful, this will overwrite the `eprint` and eprinttype` fields.
    ///
    /// `eprint` will be set to the corresponding value, and `eprinttype` will be set to the
    /// corresponding key. Returns `true` if the eprint was set, and `false` otherwise.
    fn set_eprint<Q: AsRef<str>>(&mut self, keys: Iter<'_, Q>) -> bool;

    /// Normalize whitespace by converting all whitespace blocks into a single ASCII SPACE,
    /// respecting whitespace which is explicitly escaped by `\`.
    fn normalize_whitespace(&mut self) -> bool;

    /// Strip trailing numbered series indicators, such as the (2) in `Ann. Math. (2)`
    fn strip_journal_series(&mut self) -> bool;

    /// Apply the given normalizations.
    #[inline]
    fn normalize(&mut self, nl: &Normalization) {
        if nl.normalize_whitespace {
            self.normalize_whitespace();
        }

        self.set_eprint(nl.set_eprint.iter());

        if nl.strip_journal_series {
            self.strip_journal_series();
        }
    }
}

/// Normalize whitespace by converting all blocks of consecutive whitespace into a single ASCII SPACE,
/// respecting whitespace which is explicitly escaped by `\`.
///
/// If the input requires normalization, return the new normalized string. Otherwise, the original
/// input is already normalized. Note that the returned string, if any, necessarily has a shorter
/// length than the original string.
pub fn normalize_whitespace_str(input: &str) -> Option<String> {
    /// Consume from the [`CharIndices`] as long as the input is whitespace, assuming that we
    /// previously saw a whitespace character.
    ///
    /// The offset is either the char offset immediately preceding the non-whitespace character,
    /// or the end of the input. The bool indicates if we terminated with a backslash.
    #[inline]
    fn skip_while_ws(chars: &mut CharIndices) -> (usize, bool) {
        for (offset, ch) in chars.by_ref() {
            if !ch.is_whitespace() {
                return (offset, ch == '\\');
            }
        }
        (chars.offset(), false)
    }

    /// Consume from the [`CharIndices`] as long as the input does not require normalization,
    /// assuming that we previously saw a non-whitespace character.
    ///
    /// When `skip_while_ok` terminates, it returns the maximal valid char boundary up to which
    /// point the char iterator does not require modification to normalize whitespace.
    #[inline]
    fn skip_while_ok(chars: &mut CharIndices, mut previous_was_backslash: bool) -> usize {
        let mut previous_was_unescaped_space = false;

        let final_offset = loop {
            if let Some((offset, ch)) = chars.next() {
                if previous_was_backslash {
                    previous_was_backslash = false;
                } else {
                    match ch {
                        '\\' => {
                            previous_was_backslash = true;
                        }
                        ' ' => {
                            if previous_was_unescaped_space {
                                break offset;
                            } else {
                                previous_was_unescaped_space = true;
                            }
                        }
                        ch if ch.is_whitespace() => {
                            break offset;
                        }
                        _ => previous_was_unescaped_space = false,
                    }
                }
            } else {
                break chars.offset();
            }
        };

        if previous_was_unescaped_space {
            // SAFETY: `previous_was_unescaped_space = true` only when we previously saw a space, which
            // means `final_offset >= 1`.
            unsafe { final_offset.unchecked_sub(1) }
        } else {
            final_offset
        }
    }

    /// Run a single iteration step: first, take whitespace, and then continue as far as possible.
    ///
    /// The returned index pair `(left, right)` is the next contiguous block on which the
    /// characters do not require normalization.
    #[inline]
    fn next_block_to_copy(chars: &mut CharIndices) -> (usize, usize) {
        let (left, saw_backslash) = skip_while_ws(chars);
        let right = skip_while_ok(chars, saw_backslash);
        (left, right)
    }

    let mut chars = input.char_indices();
    let mut output = String::new();

    loop {
        let (left, right) = next_block_to_copy(&mut chars);

        // short-circuit termination: no alloc required
        if left == 0 && right == input.len() {
            break None;
        }

        // the `left < right` check is necessary for the edge case of trailing whitespace,
        // which requires an extra iteration step to consume but does not result in a
        // non-trivial block to copy.
        if left < right {
            if !output.is_empty() {
                output.push(' ');
            }
            output.push_str(&input[left..right]);
        }

        if chars.offset() == input.len() {
            break Some(output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_whitespace() {
        // check short circuit
        assert_eq!(normalize_whitespace_str("a"), None);
        assert_eq!(normalize_whitespace_str("a b c"), None);
        assert_eq!(normalize_whitespace_str("a bc def gh"), None);

        // check pruning
        assert_eq!(normalize_whitespace_str("a b "), Some("a b".to_owned()));
        assert_eq!(normalize_whitespace_str(" a b"), Some("a b".to_owned()));
        assert_eq!(normalize_whitespace_str("a  b "), Some("a b".to_owned()));
        assert_eq!(normalize_whitespace_str("a\tb"), Some("a b".to_owned()));
        assert_eq!(normalize_whitespace_str("\ta b"), Some("a b".to_owned()));
        assert_eq!(normalize_whitespace_str("\t a b"), Some("a b".to_owned()));
        assert_eq!(
            normalize_whitespace_str(" \n abc b"),
            Some("abc b".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str(" \n\tad b"),
            Some("ad b".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("a\t\n\tba"),
            Some("a ba".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("aaa\t \n\tb"),
            Some("aaa b".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("a \t \n\tb"),
            Some("a b".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("a \t \n\tb\t"),
            Some("a b".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str(" aaa  b "),
            Some("aaa b".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("    a    b    "),
            Some("a b".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("   a\t   b \n   "),
            Some("a b".to_owned())
        );

        // check escapes
        assert_eq!(normalize_whitespace_str("a\\  b"), None);
        assert_eq!(normalize_whitespace_str("a\\b"), None);
        assert_eq!(normalize_whitespace_str("a\\\\ b"), None);
        assert_eq!(normalize_whitespace_str("a\\\\\\ b"), None);
        assert_eq!(normalize_whitespace_str("a\\\\\\\\ b"), None);
        assert_eq!(
            normalize_whitespace_str("a\\\\  b"),
            Some("a\\\\ b".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("a\\\\\tb"),
            Some("a\\\\ b".to_owned())
        );
        assert_eq!(normalize_whitespace_str("\\"), None);
        assert_eq!(normalize_whitespace_str("\\ "), None);

        // check edge cases
        assert_eq!(normalize_whitespace_str(""), None);
        assert_eq!(normalize_whitespace_str(" "), Some("".to_owned()));
        assert_eq!(normalize_whitespace_str("  "), Some("".to_owned()));
        assert_eq!(normalize_whitespace_str("\t"), Some("".to_owned()));
        assert_eq!(normalize_whitespace_str("\n"), Some("".to_owned()));
        assert_eq!(normalize_whitespace_str(" \t "), Some("".to_owned()));

        // check non-ASCII
        assert_eq!(normalize_whitespace_str("🍄"), None);
        assert_eq!(normalize_whitespace_str("\\\u{A0} b"), None);
        assert_eq!(
            normalize_whitespace_str("\\\u{A0} "),
            Some("\\\u{A0}".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("a\u{A0}🍄 c"),
            Some("a 🍄 c".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("a \u{A0}🍄 c"),
            Some("a 🍄 c".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("🍄 \u{A0} b c"),
            Some("🍄 b c".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("🍄\u{A0} b c"),
            Some("🍄 b c".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("\u{A0}a b 🍄"),
            Some("a b 🍄".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("\u{A0} a b c"),
            Some("a b c".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str(" \u{A0}a b c"),
            Some("a b c".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("a b c\u{A0}"),
            Some("a b c".to_owned())
        );
        assert_eq!(
            normalize_whitespace_str("a 🍄 c \u{A0}"),
            Some("a 🍄 c".to_owned())
        );
    }
}
