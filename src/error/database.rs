use thiserror::Error;

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("SQLite error: {0}")]
    SQLiteError(#[from] rusqlite::Error),
    #[error("Database missing table `{0}`")]
    TableMissing(String),
    #[error("Table `{0}` has unexpected schema:\n{1}")]
    TableIncorrectSchema(String, String),
    #[error("Citation key already exists: {0}")]
    CitationKeyExists(String),
    #[error("Citation key missing: {0}")]
    CitationKeyMissing(String),
}
