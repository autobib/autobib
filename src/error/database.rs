#[derive(Debug)]
pub enum DatabaseError {
    SQLiteError(rusqlite::Error),
    TableMissing(String),
    TableIncorrectSchema(String, String),
    CitationKeyExists(String),
    CitationKeyMissing(String),
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseError::SQLiteError(err) => err.fmt(f),
            DatabaseError::CitationKeyExists(k) => write!(f, "Citation key exists: '{k}'"),
            DatabaseError::TableMissing(table) => write!(f, "Database missing table: '{table}'"),
            DatabaseError::TableIncorrectSchema(table, schema) => {
                write!(f, "Table '{table}' has unexpected schema:\n{schema}")
            }
            DatabaseError::CitationKeyMissing(k) => write!(f, "Citation key missing: '{k}'"),
        }
    }
}

impl From<rusqlite::Error> for DatabaseError {
    fn from(err: rusqlite::Error) -> Self {
        Self::SQLiteError(err)
    }
}
