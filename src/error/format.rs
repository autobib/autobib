use thiserror::Error;

use super::RecordDataError;

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
        "Meta '%{0}' is invalid. Accepted values:\n     %entry_type %provider %sub_id %full_id"
    )]
    InvalidMeta(String),
    #[error("String started with '\"' is unclosed.")]
    UnclosedString,
    #[error("Invalid field key: {0}")]
    InvalidFieldKey(#[from] RecordDataError),
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
        "Parsed successfully, but has trailing characters.\n      Maybe this should be separated into multiple expressions?"
    )]
    Trailing(String),
    #[error("Expected {0}, but reached the end of the expression")]
    UnexpectedEof(&'static str),
    #[error("Must be non-empty.")]
    Empty,
    #[error("{0}")]
    Custom(&'static str),
}
