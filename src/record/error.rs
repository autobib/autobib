use std::fmt;

#[derive(Debug)]
pub enum RecordError {
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
                write!(f, "'{}' is not a valid source.", source)
            }
            RecordError::DatabaseFailure(error) => write!(f, "Database failure: {}", error),
            RecordError::NetworkFailure(error) => write!(f, "Network failure: {}", error),
            RecordError::Incomplete => write!(f, "Incomplete record"),
        }
    }
}
