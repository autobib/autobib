use rusqlite::{Connection, OptionalExtension, Result};

use crate::record::{lookup_record_source, Record, RecordError, RecordId, RecordSource};
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

    /// Check if database contains source:sub_id.
    pub fn contains(&self, record_id: &RecordId) -> Result<bool, RecordError> {
        let mut selector = self
            .conn
            .prepare_cached("SELECT 1 FROM records WHERE rid = ?1 LIMIT 1")?;
        let exists = selector.exists([record_id.to_string()])?;

        Ok(exists)
    }

    /// Get record for source:sub_id, returning None if the record does not exist.
    pub fn get_cached(&self, record_id: &RecordId) -> Result<Option<Record>, RecordError> {
        let mut selector = self
            .conn
            .prepare_cached("SELECT record, accessed FROM records WHERE rid = ?1")?;

        Ok(selector
            .query_row([record_id.full_id.clone()], Record::from_row)
            .optional()?)
    }

    /// Insert record_cache to source:sub_id.
    pub fn set_cached(
        &self,
        record_id: &RecordId,
        record_cache: &Record,
    ) -> Result<(), RecordError> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT OR REPLACE INTO records (rid, record, accessed) values (?1, ?2, ?3)",
        )?;

        stmt.execute(record_cache.to_param(record_id))?;

        Ok(())
    }

    /// Get the record cache associated with source:sub_id.
    pub fn get(&self, record_id: &RecordId) -> Result<Record, RecordError> {
        match self.get_cached(&record_id)? {
            Some(cached_record) => Ok(cached_record),
            None => {
                let record_source = lookup_record_source(&record_id)?;

                if record_source.is_valid_id(record_id.sub_id()) {
                    let record_cache = Record::new(record_source.get_record(record_id.sub_id())?);
                    self.set_cached(record_id, &record_cache)?;

                    Ok(record_cache)
                } else {
                    Err(RecordError::InvalidSubId(record_id.clone()))
                }
            }
        }
    }
}
