use std::fmt;

use thiserror::Error;

use super::ShortError;

#[derive(Error, Debug)]
pub struct RecordError {
    pub input: String,
    pub kind: RecordErrorKind,
}

#[derive(Debug)]
pub enum RecordErrorKind {
    Identifier(IdErrorKind),
    Alias(AliasErrorKind),
    InvalidMappedAlias(IdErrorKind),
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid key '{}': ", self.input)?;
        match &self.kind {
            RecordErrorKind::Identifier(kind) => write!(f, "{}", kind.msg()),
            RecordErrorKind::Alias(kind) => write!(f, "{}", kind.msg()),
            RecordErrorKind::InvalidMappedAlias(kind) => {
                write!(f, "auto-aliased to invalid remote id: {}", kind.msg())
            }
        }?;

        // compute and print alternative keys
        if let Some((_, sub_id)) = self.input.split_once(':') {
            let mut first = true;
            crate::provider::suggest_valid_ids(sub_id, |id| {
                if first {
                    first = false;
                    write!(f, "\n       Maybe you meant: '{id}'")
                } else {
                    write!(f, ", '{id}'")
                }
            })?;
        };
        Ok(())
    }
}

#[derive(Error, Debug)]
pub struct IdConversionError {
    pub input: String,
    pub kind: IdErrorKind,
}

#[derive(Debug)]
pub enum IdErrorKind {
    InvalidProvider,
    InvalidSubId,
    EmptyProvider,
    EmptySubId,
    IsAlias,
}

impl IdErrorKind {
    fn msg(&self) -> &'static str {
        match self {
            Self::EmptyProvider => "provider must contain non-whitespace characters",
            Self::EmptySubId => "sub-id must contain non-whitespace characters",
            Self::InvalidProvider => "provider is invalid",
            Self::InvalidSubId => "sub-id is invalid for the given provider",
            Self::IsAlias => "remote id must contain a colon",
        }
    }
}

impl ShortError for IdConversionError {
    fn short_err(&self) -> &'static str {
        self.kind.msg()
    }
}

impl fmt::Display for IdConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Invalid remote id '{}': {}",
            self.input,
            self.short_err()
        )
    }
}

impl From<IdConversionError> for RecordError {
    fn from(value: IdConversionError) -> Self {
        let IdConversionError { input, kind } = value;
        Self {
            input,
            kind: RecordErrorKind::Identifier(kind),
        }
    }
}

#[derive(Error, Debug)]
pub struct AliasConversionError {
    pub input: String,
    pub kind: AliasErrorKind,
}

#[derive(Debug)]
pub enum AliasErrorKind {
    Empty,
    IsIdentifier,
    ContainsControl,
}

impl AliasErrorKind {
    fn msg(&self) -> &'static str {
        match self {
            Self::Empty => "alias must contain non-whitespace characters",
            Self::IsIdentifier => "alias must not contain a colon",
            Self::ContainsControl => "alias must not contain control characters",
        }
    }
}

impl ShortError for AliasConversionError {
    fn short_err(&self) -> &'static str {
        self.kind.msg()
    }
}

impl fmt::Display for AliasConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid alias '{}': {}", self.input, self.short_err())
    }
}

impl From<AliasConversionError> for RecordError {
    fn from(value: AliasConversionError) -> Self {
        let AliasConversionError { input, kind } = value;
        Self {
            input,
            kind: RecordErrorKind::Alias(kind),
        }
    }
}
