use rusqlite::{Connection, OptionalExtension, Result};

use crate::record::{lookup_record_source, Record, RecordError, RecordSource, RepoId};
pub struct RecordDatabase {
    conn: Connection,
}

impl RecordDatabase {
    /// Initialize a new connection, creating the underlying database and table
    /// if the database does not yet exist.
    pub fn try_new(db_file: &str) -> Result<Self, RecordError> {
        let conn = Connection::open(db_file)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS records (
                 rid TEXT PRIMARY KEY,
                 record TEXT,
                 accessed TEXT NOT NULL
             )",
            (),
        )?;

        Ok(RecordDatabase { conn })
    }

    /// Check if database contains repo:id.
    pub fn contains(&self, repo_id: &RepoId) -> Result<bool, RecordError> {
        let mut selector = self
            .conn
            .prepare_cached("SELECT 1 FROM records WHERE rid = ?1 LIMIT 1")?;
        let exists = selector.exists([repo_id.to_string()])?;

        Ok(exists)
    }

    /// Get record for repo:id, returning None if the record does not exist.
    pub fn get_cached(&self, repo_id: &RepoId) -> Result<Option<Record>, RecordError> {
        let mut selector = self
            .conn
            .prepare_cached("SELECT record, accessed FROM records WHERE rid = ?1")?;

        Ok(selector
            .query_row([repo_id.to_string()], Record::from_row)
            .optional()?)
    }

    /// Insert record_cache to repo:id.
    pub fn set_cached(&self, repo_id: &RepoId, record_cache: &Record) -> Result<(), RecordError> {
        let mut insertor = self.conn.prepare_cached(
            "INSERT OR REPLACE INTO records (rid, record, accessed) values (?1, ?2, ?3)",
        )?;

        insertor.execute(record_cache.to_param(repo_id))?;

        Ok(())
    }

    /// Get the record cache assocated with repo:id.
    pub fn get(&self, repo_id: &RepoId) -> Result<Record, RecordError> {
        match self.get_cached(repo_id)? {
            Some(record_cache) => Ok(record_cache),
            None => {
                let record_source = lookup_record_source(repo_id)?;

                if record_source.is_valid_id(&repo_id.id) {
                    let record_cache = Record::new(record_source.get_record(&repo_id.id)?);
                    self.set_cached(repo_id, &record_cache)?;

                    Ok(record_cache)
                } else {
                    Err(RecordError::InvalidId(repo_id.clone()))
                }
            }
        }
    }
}
