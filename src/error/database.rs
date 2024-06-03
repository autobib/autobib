use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum RecordDataError {
    #[error("Key is not ASCII lowercase `[a-z]`.")]
    KeyNotAsciiLowercase,
    #[error("Key has invalid size {0}; must be at least 1 and at most `u8::MAX`.")]
    KeyInvalidLength(usize),
    #[error("Entry type is not ASCII lowercase `[a-z]`.")]
    EntryTypeNotAsciiLowercase,
    #[error("Entry type has invalid size {0}; must be at least 1 and at most `u8::MAX`.")]
    EntryTypeInvalidLength(usize),
    #[error("Value has invalid size {0}; must be at most `u16::MAX`.")]
    ValueInvalidLength(usize),
    #[error("Value does not contain balanced `{{ }}` brackets.")]
    ValueNotBalanced,
    #[error("Record data structure contains maximum number of entries.")]
    RecordDataFull,
}

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("SQLite error: {0}")]
    SQLiteError(#[from] rusqlite::Error),
    #[error("Database missing table `{0}`")]
    TableMissing(String),
    #[error("Table `{0}` has unexpected schema:\n{1}")]
    TableIncorrectSchema(String, String),
    #[error("Citation key already exists: `{0}`")]
    CitationKeyExists(String),
    #[error("Citation key missing: `{0}`")]
    CitationKeyMissing(String),
    #[error("Could not delete alias which does not exist: `{0}`")]
    AliasDeleteMissing(String),
    #[error("Citation key is null: `{0}`")]
    CitationKeyNull(String),
    #[error(transparent)]
    Data(#[from] RecordDataError),
}
