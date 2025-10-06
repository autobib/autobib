use thiserror::Error;

use super::RecordDataError;

#[derive(Error, Debug)]
pub enum KeyParseError {
    #[error(
        "Meta '%{0}' is invalid. Accepted values:\n     %entry_type %provider %sub_id %full_id"
    )]
    InvalidSpecial(String),

    #[error("Invalid field key: {0}")]
    InvalidFieldKey(#[from] RecordDataError),

    #[error("Invalid literal: {0}")]
    InvalidLiteral(#[from] serde_json::Error),
    #[error("A conditional block is missing a value.")]
    IncompleteConditional,
    #[error("Must be non-empty.")]
    Empty,
}
