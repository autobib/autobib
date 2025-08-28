use thiserror::Error;

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("SQLite error: {0}")]
    SQLiteError(#[from] rusqlite::Error),
    #[error("Error while migrating from old database (version v'{0}'): '{1}'")]
    Migration(i32, String),
    #[error(
        "Database has version newer than binary. Update `autobib` to the newest version to safely read this database.\n Database version: {0}\n Binary version: {1}"
    )]
    DatabaseVersionNewerThanBinary(i32, i32),
    #[error("Database file already exists and was modified by a different program.")]
    InvalidDatabase,
}
