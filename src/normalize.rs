//! Utilities for normalizing BibTeX data
use std::str::CharIndices;

pub trait Normalize {
    /// Attempt to set the `eprint` and `eprinttype` fields from a given field.
    ///
    /// Note that, if successful, this will overwrite the `eprint` and eprinttype` fields.
    ///
    /// `eprint` will be set to the corresponding value, and `eprinttype` will be set to the
    /// corresponding key. Returns `true` if the eprint was set, and `false` otherwise.
    fn normalize_eprint<Q: AsRef<str>>(&mut self, keys: std::slice::Iter<'_, Q>) -> bool;

    /// Normalize whitespace by converting all whitespace blocks into a single ASCII SPACE,
    /// respecting whitespace which is explicitly escaped by `\`.
    fn normalize_whitespace(&mut self) -> bool;
}

/// Normalize whitespace by converting all blocks of consecutive whitespace into a single ASCII SPACE,
/// respecting whitespace which is explicitly escaped by `\`.
///
/// If the input requires normalization, return the new normalized string. Otherwise, the original
/// input is already normalized. Note that the returned string, if any, necessarily has a shorter
/// length than the original string.
pub fn normalize_whitespace(input: &str) -> Option<String> {
    /// Consume from the [`CharIndices`] as long as the input is whitespace. Assumes that we previously
    /// saw a whitespace character.
    ///
    /// The offset is either the index immediately preceding the non-whitespace character, or the end of
    /// the input. The bool indicates if we terminated with a backslash.
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
    fn skip_while_ok(chars: &mut CharIndices, mut saw_backslash: bool) -> usize {
        let mut has_trailing_space = false;

        let final_offset = loop {
            if let Some((offset, ch)) = chars.next() {
                if saw_backslash {
                    saw_backslash = false;
                } else {
                    match ch {
                        '\\' => {
                            saw_backslash = true;
                        }
                        ' ' => {
                            if has_trailing_space {
                                break offset;
                            } else {
                                has_trailing_space = true;
                            }
                        }
                        ch if ch.is_whitespace() => {
                            break offset;
                        }
                        _ => has_trailing_space = false,
                    }
                }
            } else {
                break chars.offset();
            }
        };

        if has_trailing_space {
            // SAFETY: `has_trailing_space = true` only when we previously saw a space, which
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
    fn run_step(chars: &mut CharIndices) -> (usize, usize) {
        let (left, saw_backslash) = skip_while_ws(chars);
        let right = skip_while_ok(chars, saw_backslash);
        (left, right)
    }

    let mut chars = input.char_indices();
    let mut output = String::new();

    loop {
        let (left, right) = run_step(&mut chars);

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
        assert_eq!(normalize_whitespace("a"), None);
        assert_eq!(normalize_whitespace("a b c"), None);
        assert_eq!(normalize_whitespace("a bc def gh"), None);

        // check pruning
        assert_eq!(normalize_whitespace("a b "), Some("a b".to_owned()));
        assert_eq!(normalize_whitespace(" a b"), Some("a b".to_owned()));
        assert_eq!(normalize_whitespace("a  b "), Some("a b".to_owned()));
        assert_eq!(normalize_whitespace("a\tb"), Some("a b".to_owned()));
        assert_eq!(normalize_whitespace("\ta b"), Some("a b".to_owned()));
        assert_eq!(normalize_whitespace("\t a b"), Some("a b".to_owned()));
        assert_eq!(normalize_whitespace(" \n abc b"), Some("abc b".to_owned()));
        assert_eq!(normalize_whitespace(" \n\tad b"), Some("ad b".to_owned()));
        assert_eq!(normalize_whitespace("a\t\n\tba"), Some("a ba".to_owned()));
        assert_eq!(
            normalize_whitespace("aaa\t \n\tb"),
            Some("aaa b".to_owned())
        );
        assert_eq!(normalize_whitespace("a \t \n\tb"), Some("a b".to_owned()));
        assert_eq!(normalize_whitespace("a \t \n\tb\t"), Some("a b".to_owned()));
        assert_eq!(normalize_whitespace(" aaa  b "), Some("aaa b".to_owned()));
        assert_eq!(
            normalize_whitespace("    a    b    "),
            Some("a b".to_owned())
        );
        assert_eq!(
            normalize_whitespace("   a\t   b \n   "),
            Some("a b".to_owned())
        );

        // check escapes
        assert_eq!(normalize_whitespace("a\\  b"), None);
        assert_eq!(normalize_whitespace("a\\b"), None);
        assert_eq!(normalize_whitespace("a\\\\ b"), None);
        assert_eq!(normalize_whitespace("a\\\\\\ b"), None);
        assert_eq!(normalize_whitespace("a\\\\\\\\ b"), None);
        assert_eq!(normalize_whitespace("a\\\\  b"), Some("a\\\\ b".to_owned()));
        assert_eq!(normalize_whitespace("a\\\\\tb"), Some("a\\\\ b".to_owned()));

        // check edge cases
        assert_eq!(normalize_whitespace(""), None);
        assert_eq!(normalize_whitespace(" "), Some("".to_owned()));
        assert_eq!(normalize_whitespace("  "), Some("".to_owned()));
        assert_eq!(normalize_whitespace("\t"), Some("".to_owned()));
        assert_eq!(normalize_whitespace("\n"), Some("".to_owned()));
        assert_eq!(normalize_whitespace(" \t "), Some("".to_owned()));

        // check non-ASCII
        assert_eq!(normalize_whitespace("🍄"), None);
        assert_eq!(normalize_whitespace("\\\u{A0} b"), None);
        assert_eq!(
            normalize_whitespace("\\\u{A0} "),
            Some("\\\u{A0}".to_owned())
        );
        assert_eq!(
            normalize_whitespace("a\u{A0}🍄 c"),
            Some("a 🍄 c".to_owned())
        );
        assert_eq!(
            normalize_whitespace("a \u{A0}🍄 c"),
            Some("a 🍄 c".to_owned())
        );
        assert_eq!(
            normalize_whitespace("🍄 \u{A0} b c"),
            Some("🍄 b c".to_owned())
        );
        assert_eq!(
            normalize_whitespace("🍄\u{A0} b c"),
            Some("🍄 b c".to_owned())
        );
        assert_eq!(
            normalize_whitespace("\u{A0}a b 🍄"),
            Some("a b 🍄".to_owned())
        );
        assert_eq!(
            normalize_whitespace("\u{A0} a b c"),
            Some("a b c".to_owned())
        );
        assert_eq!(
            normalize_whitespace(" \u{A0}a b c"),
            Some("a b c".to_owned())
        );
        assert_eq!(
            normalize_whitespace("a b c\u{A0}"),
            Some("a b c".to_owned())
        );
        assert_eq!(
            normalize_whitespace("a 🍄 c \u{A0}"),
            Some("a 🍄 c".to_owned())
        );
    }
}