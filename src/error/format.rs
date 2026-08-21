use thiserror::Error;

use autobib_entry::error::DataError;

struct TrailingChars<'a>(&'a str);

impl std::fmt::Display for TrailingChars<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut chars = self.0.chars();
        for ch in chars.by_ref().take(4) {
            ch.fmt(f)?;
        }
        if chars.next().is_some() {
            f.write_str("..")?;
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
pub struct KeyParseError {
    pub kind: KeyParseErrorKind,
    pub span: Option<std::ops::Range<usize>>,
}

impl std::fmt::Display for KeyParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.kind.fmt(f)
    }
}

#[derive(Error, Debug)]
pub enum KeyParseErrorKind {
    #[error(
        "Meta '%{0}' is invalid. Accepted values:\n     %entry_type %provider %sub_id %full_id %key %modified %json"
    )]
    InvalidMeta(String),
    #[error("String started with '\"' is unclosed.")]
    UnclosedString,
    #[error("Invalid field key: {0}")]
    InvalidFieldKey(#[from] DataError),
    #[error("No closing bracket to match '('")]
    MissingBracket,
    #[error("No opening bracket to match ')'")]
    ExtraBracket,
    #[error("Unexpected character: {0}")]
    UnexpectedChar(char),
    #[error("Invalid JSON literal")]
    InvalidLiteral,
    #[error("A conditional block is missing a value.")]
    IncompleteConditional,
    #[error("Expected {0}, received {1}")]
    Unexpected(&'static str, &'static str),
    #[error(
        "Parsed successfully, but has trailing characters '{}'.\n      Maybe this should be separated into multiple expressions?",
        TrailingChars(.0)
    )]
    Trailing(String),
    #[error("Expected {0}, but reached the end of the expression")]
    UnexpectedEof(&'static str),
}
