use reqwest::StatusCode;
use thiserror::Error;

use super::RecordDataError;

#[derive(Error, Debug)]
pub enum ProviderError {
    #[error("Network failure: {0}")]
    NetworkFailure(#[from] reqwest::Error),
    #[error("Undefined local record: {0}")]
    UndefinedLocal(String),
    #[error("Unexpected status code: {0}")]
    UnexpectedStatusCode(StatusCode),
    #[error("Unexpected failure: {0}")]
    Unexpected(String),
    #[error("Incompatible data format: {0}")]
    Format(#[from] RecordDataError),
}
