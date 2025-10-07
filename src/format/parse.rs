use std::ops::Range;

use crate::error::{KeyParseError, KeyParseErrorKind};

/// A lexer for an expression.
pub struct Lexer<'a> {
    /// no leading or trailing whitespace
    inner: &'a str,
    offset: usize,
}

#[derive(Debug, PartialEq)]
pub struct Token<'a> {
    pub span: Range<usize>,
    pub kind: Kind<'a>,
}

#[derive(Debug, PartialEq)]
pub enum Kind<'a> {
    Whitespace,
    /// ?
    Opt,
    /// =
    Cond,
    /// !
    Neg,
    /// %
    Meta,
    /// [a-zA-Z0-9_] or (ident)
    Ident(&'a str),
    /// "string"
    String(String),
}

impl<'a> Kind<'a> {
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

fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

impl<'a> Lexer<'a> {
    pub fn new(inner: &'a str) -> Self {
        Self { inner, offset: 0 }
    }

    /// The remaining (unparsed) chars in this lexer
    pub fn remainder(&self) -> &'a str {
        &self.inner[self.offset..]
    }

    fn skip_if_char(&mut self, exp: char) -> bool {
        let mut chars = self.remainder().chars();
        if chars.next().is_some_and(|ch| ch == exp) {
            self.offset += exp.len_utf8();
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn skip_if_opt(&mut self) -> bool {
        self.skip_if_char('?')
    }

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

    fn step(&mut self, increment: usize, kind: Kind<'a>) -> Token<'a> {
        let new_offset = self.offset + increment;
        let span = self.offset..new_offset;
        self.offset = new_offset;
        Token { kind, span }
    }

    fn step_err(&mut self, increment: usize, kind: KeyParseErrorKind) -> KeyParseError {
        let new_offset = self.offset + increment;
        let span = self.offset..new_offset;
        self.offset = new_offset;
        KeyParseError {
            kind,
            span: Some(span),
        }
    }

    fn step_err_end(&mut self, kind: KeyParseErrorKind) -> KeyParseError {
        let new_offset = self.inner.len();
        let span = self.offset..new_offset;
        self.offset = new_offset;
        KeyParseError {
            kind,
            span: Some(span),
        }
    }

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

    pub fn expect_token(&mut self, msg: &'static str) -> Result<Token<'a>, KeyParseError> {
        match self.next_token()? {
            Some(token) => Ok(token),
            None => Err(KeyParseError {
                kind: KeyParseErrorKind::UnexpectedEof(msg),
                span: None,
            }),
        }
    }

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
                        return Ok(Some(self.step(cutoff, Kind::String(s))));
                    }
                }
                Err(self.step_err_end(KeyParseErrorKind::UnclosedString))
            }
            '=' => Ok(Some(self.step(1, Kind::Cond))),
            '!' => Ok(Some(self.step(1, Kind::Neg))),
            '%' => Ok(Some(self.step(1, Kind::Meta))),
            '?' => Ok(Some(self.step(1, Kind::Opt))),
            '(' => {
                let tail = chars.as_str().as_bytes();
                match memchr::memchr(b')', tail) {
                    Some(idx) => {
                        let cutoff = idx + 1;
                        let escaped = &self.remainder()[1..cutoff];
                        Ok(Some(self.step(cutoff + 1, Kind::Ident(escaped))))
                    }
                    None => Err(self.step_err_end(KeyParseErrorKind::MissingBracket)),
                }
            }
            ')' => Err(self.step_err(1, KeyParseErrorKind::ExtraBracket)),
            ch if ch.is_whitespace() => {
                let extra = self.remainder().len() - self.remainder().trim_start().len();
                Ok(Some(self.step(extra, Kind::Whitespace)))
            }
            ch if is_ident_char(ch) => match self.remainder().find(|ch| !is_ident_char(ch)) {
                Some(cutoff) => {
                    let res = &self.remainder()[..cutoff];
                    Ok(Some(self.step(cutoff, Kind::Ident(res))))
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
            },
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
