use thiserror::Error;

use super::InvalidBytesError;

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("SQLite error: {0}")]
    SQLiteError(#[from] rusqlite::Error),
    #[error("Database missing table '{0}'")]
    TableMissing(String),
    #[error("Table '{0}' has unexpected schema:\n{1}")]
    TableIncorrectSchema(String, String),
    #[error("Database has invalid schema version: {0}")]
    InvalidSchemaVersion(i64),
}

#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("SQLite error: {0}")]
    SQLiteError(#[from] rusqlite::Error),
    #[error("Record with canonical id '{0}' has error in binary format:\n  {1}")]
    MalformedRecordData(String, InvalidBytesError),
    #[error("Internal consistency error(s):{0}")]
    ConsistencyError(String),
    #[error("Record row '{0}' present in records table but not in citation keys table")]
    DanglingRecord(String),
}
