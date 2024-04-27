use reqwest::StatusCode;
use std::fmt;

#[derive(Debug)]
pub enum RecordError {
    UnexpectedFailure(String),
    UnexpectedStatusCode(StatusCode),
    InvalidSource(String),
    NetworkFailure(reqwest::Error),
    DatabaseFailure(rusqlite::Error),
    Incomplete,
}

impl From<rusqlite::Error> for RecordError {
    fn from(err: rusqlite::Error) -> Self {
        RecordError::DatabaseFailure(err)
    }
}

impl From<reqwest::Error> for RecordError {
    fn from(err: reqwest::Error) -> Self {
        RecordError::NetworkFailure(err)
    }
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordError::InvalidSource(source) => {
                write!(f, "Invalid source: '{source}'")
            }
            RecordError::DatabaseFailure(error) => write!(f, "Database failure: {error}"),
            RecordError::UnexpectedFailure(reason) => write!(f, "Unexpected failure: {reason}"),
            RecordError::UnexpectedStatusCode(code) => {
                write!(f, "Unexpected status code: {code}")
            }
            RecordError::NetworkFailure(error) => write!(f, "Network failure: {error}"),
            RecordError::Incomplete => write!(f, "Incomplete record"),
        }
    }
}
