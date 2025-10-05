use thiserror::Error;

use super::RecordDataError;

#[derive(Error, Debug)]
pub enum KeyParseError {
    #[error(
        "Specifier '%{0}' does not exist. Accepted values are: %entry_type, %provider, %sub_id, %full_id"
    )]
    InvalidSpecial(String),

    #[error("Invalid field key: {0}")]
    InvalidFieldKey(#[from] RecordDataError),

    #[error("Invalid literal: {0}")]
    InvalidLiteral(#[from] serde_json::Error),
    #[error("A conditional block is missing a value.")]
    IncompleteConditional,
}
