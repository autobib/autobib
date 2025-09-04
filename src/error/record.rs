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
        }?;

        // compute and print alternative keys
        if let Some((_, sub_id)) = self.input.split_once(':') {
            let mut first = true;
            crate::provider::suggest_valid_remote_identifiers(sub_id, |remote_id| {
                if first {
                    first = false;
                    write!(f, "\n       Maybe you meant: '{remote_id}'")
                } else {
                    write!(f, ", '{remote_id}'")
                }
            })?;
        };
        Ok(())
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
            Self::EmptyProvider => "provider must contain non-whitespace characters",
            Self::EmptySubId => "sub-id must contain non-whitespace characters",
            Self::InvalidProvider => "provider is invalid",
            Self::InvalidSubId => "sub-id is invalid for the given provider",
            Self::IsAlias => "remote id must contain a colon",
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
        Self {
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
            Self::Empty => "alias must contain non-whitespace characters",
            Self::IsRemoteId => "alias must not contain a colon",
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
