use crate::record::{lookup_record_source, RecordError, RecordId, RecordSource};
use biblatex::Entry;
use chrono::{DateTime, Local};
use rusqlite::{Connection, OptionalExtension, Result, Transaction};
use std::string::ToString;

pub struct RecordDatabase {
    conn: Connection,
}

impl RecordDatabase {
    /// Create the underlying database and tables
    pub fn create(db_file: &str) -> Result<Self, RecordError> {
        let conn = Connection::open(db_file)?;

        // Table to store records
        conn.execute(
            "CREATE TABLE Records (
                 key INTEGER PRIMARY KEY,
                 record_id TEXT,
                 data TEXT,
                 modified TEXT NOT NULL
             )",
            (),
        )?;

        // Table to store citation keys
        conn.execute(
            "CREATE TABLE CitationKeys (
                 name TEXT NOT NULL PRIMARY KEY,
                 record_key INTEGER,
                 FOREIGN KEY(record_key) REFERENCES Records(key)
             )",
            (),
        )?;

        // Table to store records which do not exist
        conn.execute(
            "CREATE TABLE NullRecords (
                 record_id TEXT PRIMARY KEY,
                 attempted TEXT NOT NULL
             )",
            (),
        )?;

        Ok(RecordDatabase { conn })
    }

    /// Initialize a new connection, creating the underlying database and table
    /// if the database does not yet exist.
    pub fn open(db_file: &str) -> Result<Self, rusqlite::Error> {
        // TODO: validation for consistency?
        Ok(RecordDatabase {
            conn: Connection::open(db_file)?,
        })
    }

    /// Check if database contains "<source>:<sub_id>".
    pub fn key(&self, record_id: &RecordId) -> Result<Option<usize>, rusqlite::Error> {
        let mut selector = self
            .conn
            .prepare_cached("SELECT record_key FROM CitationKeys WHERE name = ?1")?;

        selector
            .query_row([record_id.to_string()], |row| row.get("record_key"))
            .optional()
    }

    /// Get response for "<source>:<sub_id>", or NotFound.
    pub fn get_cached_data(&self, record_id: RecordId) -> Result<CacheResponse, rusqlite::Error> {
        // First, try to get the key from CitationKeys
        match self.key(&record_id) {
            // If the key exists, get the corresponding record
            Ok(Some(key)) => {
                let mut record_selector = self
                    .conn
                    .prepare_cached("SELECT modified, data FROM Records WHERE key = ?1")?;
                let mut record_rows = record_selector.query([key])?;

                match record_rows.next() {
                    // Valid record
                    Ok(Some(row)) => Self::cache_response_from_row(record_id, row),
                    Ok(None) => {
                        panic!("A key in CitationKeys does not correspond to a row in Records!")
                    }
                    Err(err) => Err(err),
                }
            }
            // No key, check for cache in NullRecords
            Ok(None) => {
                let mut null_selector = self
                    .conn
                    .prepare_cached("SELECT attempted FROM NullRecords WHERE record_id = ?1")?;
                let mut null_rows = null_selector.query([&record_id.full_id()])?;

                match null_rows.next() {
                    // Cached null
                    Ok(Some(row)) => Ok(CacheResponse::Found(Record {
                        id: record_id,
                        data: None,
                        modified: row.get("attempted")?,
                    })),
                    Ok(None) => Ok(CacheResponse::NotFound(record_id)),
                    Err(err) => Err(err),
                }
            }
            Err(err) => Err(err),
        }
    }

    fn cache_response_from_row(
        record_id: RecordId,
        row: &rusqlite::Row,
    ) -> Result<CacheResponse, rusqlite::Error> {
        let data_str: String = row.get("data")?;
        let modified: DateTime<Local> = row.get("modified")?;

        Ok(CacheResponse::Found(Record {
            id: record_id,
            data: serde_json::from_str(&data_str).unwrap(),
            modified,
        }))
    }

    fn perform_set_transaction<'a>(
        tx: &mut Transaction<'a>,
        record: &Record,
    ) -> Result<(), rusqlite::Error> {
        match &record.data {
            // if there is data to insert, insert it
            Some(entry) => {
                let mut setter = tx.prepare_cached(
                    "INSERT OR REPLACE INTO Records (record_id, data, modified) values (?1, ?2, ?3)",
                    )?;
                setter.execute((
                    record.id.full_id(),
                    serde_json::to_string(entry).unwrap(), // TODO: proper error here
                    &record.modified,
                ))?;

                // get identifier
                let key = tx.last_insert_rowid();

                // add citation keys
                let mut key_writer = tx.prepare_cached(
                    "INSERT OR REPLACE INTO CitationKeys (name, record_key) values (?1, ?2)",
                )?;
                key_writer.execute((record.id.full_id(), key))?;
            }
            // otherwise, cache the Null
            None => {
                let mut setter = tx.prepare_cached(
                    "INSERT OR REPLACE INTO NullRecords (record_id, attempted) values (?1, ?2)",
                )?;
                setter.execute((record.id.full_id(), Local::now()))?;
            }
        }
        Ok(())
    }

    /// Insert record to the Records table and use the resulting key to set the corresponding
    /// CitationKeys entry.
    pub fn set_cached_data(&mut self, record: &Record) -> Result<(), rusqlite::Error> {
        let mut tx = self.conn.transaction()?;

        Self::perform_set_transaction(&mut tx, &record)?;

        tx.commit()
    }

    /// Get the record associated with record_id
    pub fn get(&mut self, record_id: RecordId) -> Result<Record, RecordError> {
        match self.get_cached_data(record_id)? {
            CacheResponse::Found(cached_record) => Ok(cached_record),
            CacheResponse::NotFound(record_id) => {
                let record_source = lookup_record_source(&record_id)?;

                if record_source.is_valid_id(record_id.sub_id()) {
                    match record_source.get_record(record_id.sub_id()) {
                        Ok(Some(entry)) => {
                            let record = Record::new(record_id, Some(entry));
                            self.set_cached_data(&record)?;
                            Ok(record)
                        }
                        Ok(None) => Ok(Record::new(record_id, None)),
                        Err(err) => Err(err),
                    }
                } else {
                    Err(RecordError::InvalidSubId(record_id))
                }
            }
        }
    }
}

pub struct Record {
    pub id: RecordId,
    pub data: Option<Entry>,
    pub modified: DateTime<Local>,
}

impl Record {
    pub fn new(id: RecordId, data: Option<Entry>) -> Self {
        Self {
            id,
            data,
            modified: Local::now(),
        }
    }
}

pub enum CacheResponse {
    Found(Record),
    NotFound(RecordId),
}
