use reqwest::StatusCode;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SourceError {
    #[error("Network failure: {0}")]
    NetworkFailure(#[from] reqwest::Error),
    #[error("Unexpected status code: {0}")]
    UnexpectedStatusCode(StatusCode),
    #[error("Unexpected failure: {0}")]
    Unexpected(String),
}
