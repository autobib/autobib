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
    RemoteId(RemoteIdErrorKind),
    Alias(AliasErrorKind),
    InvalidMappedAlias(RemoteIdErrorKind),
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid key '{}': ", self.input)?;
        match &self.kind {
            RecordErrorKind::RemoteId(kind) => write!(f, "{}", kind.msg()),
            RecordErrorKind::Alias(kind) => write!(f, "{}", kind.msg()),
            RecordErrorKind::InvalidMappedAlias(kind) => {
                write!(f, "auto-aliased to invalid remote id: {}", kind.msg())
            }
        }
    }
}

#[derive(Error, Debug)]
pub struct RemoteIdConversionError {
    pub input: String,
    pub kind: RemoteIdErrorKind,
}

#[derive(Debug)]
pub enum RemoteIdErrorKind {
    InvalidProvider,
    InvalidSubId,
    EmptyProvider,
    EmptySubId,
    IsAlias,
}

impl RemoteIdErrorKind {
    fn msg(&self) -> &'static str {
        match self {
            RemoteIdErrorKind::EmptyProvider => "provider must contain non-whitespace characters",
            RemoteIdErrorKind::EmptySubId => "sub-id must contain non-whitespace characters",
            RemoteIdErrorKind::InvalidProvider => "provider is invalid",
            RemoteIdErrorKind::InvalidSubId => "sub-id is invalid for the given provider",
            RemoteIdErrorKind::IsAlias => "remote id must contain a colon",
        }
    }
}

impl ShortError for RemoteIdConversionError {
    fn short_err(&self) -> &'static str {
        self.kind.msg()
    }
}

impl fmt::Display for RemoteIdConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Invalid remote id '{}': {}",
            self.input,
            self.short_err()
        )
    }
}

impl From<RemoteIdConversionError> for RecordError {
    fn from(value: RemoteIdConversionError) -> Self {
        let RemoteIdConversionError { input, kind } = value;
        RecordError {
            input,
            kind: RecordErrorKind::RemoteId(kind),
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
    IsRemoteId,
}

impl AliasErrorKind {
    fn msg(&self) -> &'static str {
        match self {
            AliasErrorKind::Empty => "alias must contain non-whitespace characters",
            AliasErrorKind::IsRemoteId => "alias must not contain a colon",
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
        RecordError {
            input,
            kind: RecordErrorKind::Alias(kind),
        }
    }
}
