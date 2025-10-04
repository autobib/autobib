use thiserror::Error;
use ureq::http::StatusCode;

use super::{RecordDataError, RecordError};

#[derive(Error, Debug)]
pub enum ProviderError {
    #[error("Reference source returned an invalid identifier: '{0}'")]
    InvalidIdFromProvider(String),
    #[error("Reference source returned a key corresponding to a null record: '{0}'")]
    UnexpectedNullRemoteFromProvider(String),
    #[error("Network failure: {0}")]
    NetworkFailure(#[from] ureq::Error),
    #[error("Unexpected local record '{0}'")]
    UnexpectedLocal(String),
    #[error(
        "API server is temporarily inaccessible; try again later. If this is a recurring problem, please report it at https://github.com/autobib/autobib/issues"
    )]
    TemporaryFailure,
    #[error("Unexpected status code {0}")]
    UnexpectedStatusCode(StatusCode),
    #[error(
        "API returned response in unexpected format. This is a bug, please report it at 'https://github.com/autobib/autobib/issues', including the below error message:\n> {0}"
    )]
    UnexpectedResponseFormat(String),
    #[error("Unexpected failure: {0}")]
    Unexpected(String),
    #[error("Incompatible data format: {0}")]
    Format(#[from] RecordDataError),
}

impl From<RecordError> for ProviderError {
    fn from(err: RecordError) -> Self {
        let RecordError { input, .. } = err;
        Self::InvalidIdFromProvider(input)
    }
}

impl From<ureq::http::Error> for ProviderError {
    fn from(value: ureq::http::Error) -> Self {
        Self::NetworkFailure(ureq::Error::Http(value))
    }
}
