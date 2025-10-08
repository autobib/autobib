use std::ops::Range;

use crate::error::{KeyParseError, KeyParseErrorKind};

/// A lexer for an expression.
pub struct Lexer<'a> {
    /// no leading or trailing whitespace
    inner: &'a str,
    /// the index into the offset
    offset: usize,
}

/// A single token produced by the lexer.
#[derive(Debug, PartialEq)]
pub struct Token<'a> {
    /// The span in the source str from which this token originated.
    pub span: Range<usize>,
    /// The kind of token.
    pub kind: Kind<'a>,
}

/// A token kind
#[derive(Debug, PartialEq)]
pub enum Kind<'a> {
    /// Consecutive Unicode whitespace
    Whitespace,
    /// The '?' character
    Opt,
    /// The '=' character
    Cond,
    /// The '!' character
    Neg,
    /// The '%' character
    Meta,
    /// Either a bare identifier in the range `[a-zA-Z0-9_]` or a bracketed identifier `(ident)`
    /// where `ident` does not contain closing brackets.
    Ident(&'a str),
    /// A JSON string, like `"string"`.
    String(String),
}

impl<'a> Kind<'a> {
    /// Returns a short human-readable description of the kind.
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Whitespace => "whitespace",
            Self::Opt => "an optional modifier (?)",
            Self::Cond => "a conditional marker (=)",
            Self::Neg => "a negation marker (!)",
            Self::Meta => "a meta marker (%)",
            Self::Ident(_) => "an identifier",
            Self::String(_) => "a string",
        }
    }
}

/// Returns of a character is permitted in a bare identifier.
fn is_bare_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

impl<'a> Lexer<'a> {
    /// Create a new lexer with an inner source string.
    pub fn new(inner: &'a str) -> Self {
        Self { inner, offset: 0 }
    }

    /// The remaining (unparsed) chars in this lexer
    pub fn remainder(&self) -> &'a str {
        &self.inner[self.offset..]
    }

    /// Consume the next token if it is a `?` token.
    ///
    /// Returns `Some` if a token was consumed, and `None` otherwise.
    #[inline]
    pub fn skip_if_opt(&mut self) -> Option<()> {
        match self.inner.as_bytes().get(self.offset) {
            Some(b'?') => {
                self.offset += 1;
                Some(())
            }
            _ => None,
        }
    }

    /// Consume the next token if it is a `!` or `=` token.
    ///
    /// Returns `Some(true)` if a `=` token was consumed, `Some(false)` if a `=` token was
    /// consumed, and `None` otherwise.
    #[inline]
    pub fn skip_if_cond(&mut self) -> Option<bool> {
        match self.inner.as_bytes().get(self.offset) {
            Some(b'!') => {
                self.offset += 1;
                Some(false)
            }
            Some(b'=') => {
                self.offset += 1;
                Some(true)
            }
            _ => None,
        }
    }

    /// A helper function to yield a token and increment the inner offset.
    fn step_ok(&mut self, increment: usize, kind: Kind<'a>) -> Token<'a> {
        let new_offset = self.offset + increment;
        let span = self.offset..new_offset;
        self.offset = new_offset;
        Token { kind, span }
    }

    /// A helper function to yield an error and increment the inner offset.
    fn step_err(&mut self, increment: usize, kind: KeyParseErrorKind) -> KeyParseError {
        let new_offset = self.offset + increment;
        let span = self.offset..new_offset;
        self.offset = new_offset;
        KeyParseError {
            kind,
            span: Some(span),
        }
    }

    /// A helper function to yield an error and increment the inner offset if the error results in
    /// consuming the entire internal buffer.
    ///
    /// The corresponding span is the remainder of the expression.
    fn step_err_final(&mut self, kind: KeyParseErrorKind) -> KeyParseError {
        let new_offset = self.inner.len();
        let span = self.offset..new_offset;
        self.offset = new_offset;
        KeyParseError {
            kind,
            span: Some(span),
        }
    }

    /// Expect the end of the expression.
    ///
    /// Returns an error if there are any trailing characters.
    pub fn expect_eof(&mut self) -> Result<(), KeyParseError> {
        let rem = self.remainder();
        if rem.is_empty() {
            Ok(())
        } else {
            Err(KeyParseError {
                kind: KeyParseErrorKind::Trailing(rem.into()),
                span: Some(self.offset..self.inner.len()),
            })
        }
    }

    /// Expect a token.
    ///
    /// Returns an error if there are no more tokens.
    pub fn expect_token(&mut self, msg: &'static str) -> Result<Token<'a>, KeyParseError> {
        match self.next_token()? {
            Some(token) => Ok(token),
            None => Err(KeyParseError {
                kind: KeyParseErrorKind::UnexpectedEof(msg),
                span: None,
            }),
        }
    }

    /// Read the next token from the expression, if any.
    pub fn next_token(&mut self) -> Result<Option<Token<'a>>, KeyParseError> {
        let mut chars = self.remainder().chars();
        let c = match chars.next() {
            Some(c) => c,
            None => return Ok(None),
        };
        match c {
            '"' => {
                let tail = chars.as_str().as_bytes();
                for idx in memchr::memchr_iter(b'"', tail) {
                    if idx == 0 || tail[idx - 1] != b'\\' {
                        let cutoff = idx + 2;
                        let s = match serde_json::from_str(&self.remainder()[..cutoff]) {
                            Ok(s) => s,
                            Err(_) => todo!(),
                        };
                        return Ok(Some(self.step_ok(cutoff, Kind::String(s))));
                    }
                }
                Err(self.step_err_final(KeyParseErrorKind::UnclosedString))
            }
            '=' => Ok(Some(self.step_ok(1, Kind::Cond))),
            '!' => Ok(Some(self.step_ok(1, Kind::Neg))),
            '%' => Ok(Some(self.step_ok(1, Kind::Meta))),
            '?' => Ok(Some(self.step_ok(1, Kind::Opt))),
            '(' => {
                let tail = chars.as_str().as_bytes();
                match memchr::memchr(b')', tail) {
                    Some(idx) => {
                        let cutoff = idx + 1;
                        let escaped = &self.remainder()[1..cutoff];
                        Ok(Some(self.step_ok(cutoff + 1, Kind::Ident(escaped))))
                    }
                    None => Err(self.step_err_final(KeyParseErrorKind::MissingBracket)),
                }
            }
            ')' => Err(self.step_err(1, KeyParseErrorKind::ExtraBracket)),
            ch if ch.is_whitespace() => {
                let extra = self.remainder().len() - self.remainder().trim_start().len();
                Ok(Some(self.step_ok(extra, Kind::Whitespace)))
            }
            ch if is_bare_ident_char(ch) => {
                match self.remainder().find(|ch| !is_bare_ident_char(ch)) {
                    Some(cutoff) => {
                        let res = &self.remainder()[..cutoff];
                        Ok(Some(self.step_ok(cutoff, Kind::Ident(res))))
                    }
                    None => {
                        let res = self.remainder();
                        let start = self.offset;
                        self.offset = self.inner.len();
                        Ok(Some(Token {
                            span: start..self.offset,
                            kind: Kind::Ident(res),
                        }))
                    }
                }
            }
            ch => Err(self.step_err(ch.len_utf8(), KeyParseErrorKind::UnexpectedChar(ch))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexer() {
        fn check(lexer: &mut Lexer<'_>, span: Range<usize>, kind: Kind<'_>) {
            assert_eq!(lexer.next_token().unwrap().unwrap(), Token { span, kind });
        }

        let s = r#""A str\"ing" next(??)%?==more_c"#;
        let mut lexer = Lexer::new(s);
        check(&mut lexer, 0..12, Kind::String("A str\"ing".into()));
        check(&mut lexer, 12..13, Kind::Whitespace);
        check(&mut lexer, 13..17, Kind::Ident("next"));
        check(&mut lexer, 17..21, Kind::Ident("??"));
        check(&mut lexer, 21..22, Kind::Meta);
        check(&mut lexer, 22..23, Kind::Opt);
        check(&mut lexer, 23..24, Kind::Cond);
        check(&mut lexer, 24..25, Kind::Cond);
        check(&mut lexer, 25..31, Kind::Ident("more_c"));
        assert!(lexer.next_token().unwrap().is_none());
    }
}
