use crate::record::{lookup_record_source, Record, RecordError, RecordId, RecordSource};
use chrono::{DateTime, Local};
use rusqlite::{Connection, Result};
use std::string::ToString;

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

    /// Check if database contains "<source>:<sub_id>".
    pub fn contains(&self, record_id: &RecordId) -> Result<bool, RecordError> {
        let mut selector = self
            .conn
            .prepare_cached("SELECT 1 FROM records WHERE rid = ?1 LIMIT 1")?;
        let exists = selector.exists([record_id.to_string()])?;

        Ok(exists)
    }

    /// Get record for "<source>:<sub_id>", returning None if the record does not exist.
    pub fn get_cached(&self, record_id: RecordId) -> Result<CacheResponse, RecordError> {
        let mut selector = self
            .conn
            .prepare_cached("SELECT record, accessed FROM records WHERE rid = ?1")?;
        // TODO: The new String allocation can be avoided if the query is completed before the move of record_id.
        //       Right now, those two things happen in a single function call.
        let query_result = selector.query_row([String::from(record_id.full_id())], |row| {
            Self::record_from_row(record_id, row)
        });
        Ok(CacheResponse::Found(query_result?))
    }

    fn record_from_row(
        record_id: RecordId,
        row: &rusqlite::Row,
    ) -> Result<Record, rusqlite::Error> {
        let record_cache_str: String = row.get("record")?;
        let retrieved: DateTime<Local> = row.get("accessed")?;
        Ok(Record {
            id: record_id,
            entry: serde_json::from_str(&record_cache_str).unwrap(),
            retrieved,
        })
    }

    /// Insert record to the "records" table.
    pub fn set_cached(&self, record_cache: &Record) -> Result<(), RecordError> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT OR REPLACE INTO records (rid, record, accessed) values (?1, ?2, ?3)",
        )?;

        stmt.execute(Self::params_from_record(record_cache))?;

        Ok(())
    }

    fn params_from_record<'a>(record: &'a Record) -> (&'a str, String, &'a DateTime<Local>) {
        // TODO: proper error here
        (
            record.id.full_id(),
            serde_json::to_string(&record.entry).unwrap(),
            &record.retrieved,
        )
    }

    /// Get the record cache associated with "<source>:<sub_id>".
    pub fn get(&self, record_id: RecordId) -> Result<Record, RecordError> {
        match self.get_cached(record_id)? {
            CacheResponse::Found(cached_record) => Ok(cached_record),
            CacheResponse::NotFound(record_id) => {
                let record_source = lookup_record_source(&record_id)?;

                if record_source.is_valid_id(record_id.sub_id()) {
                    let entry = record_source.get_record(record_id.sub_id())?;
                    let record = Record::new(record_id, entry);
                    self.set_cached(&record)?;
                    Ok(record)
                } else {
                    Err(RecordError::InvalidSubId(record_id))
                }
            }
        }
    }
}

pub enum CacheResponse {
    Found(Record),
    NotFound(RecordId),
}
