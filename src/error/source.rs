use reqwest::StatusCode;
use std::fmt;

#[derive(Debug)]
pub enum SourceError {
    NetworkFailure(reqwest::Error),
    UnexpectedStatusCode(StatusCode),
    Unexpected(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::Unexpected(reason) => write!(f, "Unexpected failure: {reason}"),
            SourceError::UnexpectedStatusCode(code) => {
                write!(f, "Unexpected status code: {code}")
            }
            SourceError::NetworkFailure(error) => write!(f, "Network failure: {error}"),
        }
    }
}

impl From<reqwest::Error> for SourceError {
    fn from(err: reqwest::Error) -> Self {
        Self::NetworkFailure(err)
    }
}
