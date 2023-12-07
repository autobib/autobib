use crate::record::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Result, Transaction};

pub struct RecordDatabase {
    conn: Connection,
}

impl RecordDatabase {

    fn initialize_database<'a>(tx: &Transaction<'a>) -> Result<(), rusqlite::Error> {
        // Table to store records
        tx.execute(
            "CREATE TABLE Records (
                 key INTEGER PRIMARY KEY,
                 record_id TEXT NOT NULL,
                 data TEXT NOT NULL,
                 modified TEXT NOT NULL
             )",
            (),
        )?;

        // Table to store citation keys
        tx.execute(
            "CREATE TABLE CitationKeys (
                 name TEXT NOT NULL PRIMARY KEY,
                 record_key INTEGER,
                 FOREIGN KEY(record_key) REFERENCES Records(key) ON DELETE CASCADE
             )",
            (),
        )?;

        // Table to store records which do not exist
        tx.execute(
            "CREATE TABLE NullRecords (
                 record_id TEXT NOT NULL PRIMARY KEY,
                 attempted TEXT NOT NULL
             )",
            (),
        )?;

        Ok(())
    }

    /// Create the underlying database and tables
    pub fn create(db_file: &str) -> Result<Self, rusqlite::Error> {
        let mut conn = Connection::open(db_file)?;

        let tx = conn.transaction()?;
        Self::initialize_database(&tx)?;
        tx.commit()?;

        Ok(RecordDatabase { conn })
    }

    /// Initialize a new connection, creating the underlying database and table
    /// if the database does not yet exist.
    pub fn open(db_file: &str) -> Result<Self, rusqlite::Error> {
        // TODO: validation for consistency?
        Ok(RecordDatabase {
            conn: Connection::open_with_flags(
                db_file,
                OpenFlags::SQLITE_OPEN_READ_WRITE
                    | OpenFlags::SQLITE_OPEN_URI
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?,
        })
    }

    /// Check if database contains '\<source\>:\<sub_id\>'.
    fn get_record_key(&self, record_id: &RecordId) -> Result<Option<usize>, rusqlite::Error> {
        let mut selector = self
            .conn
            .prepare_cached("SELECT record_key FROM CitationKeys WHERE name = ?1")?;

        selector
            .query_row([record_id.full_id()], |row| row.get("record_key"))
            .optional()
    }

    /// Get response for '\<source\>:\<sub_id\>', or NotFound.
    pub fn get_cached_data(&self, record_id: RecordId) -> Result<CacheResponse, rusqlite::Error> {
        // First, try to get the key from CitationKeys
        match self.get_record_key(&record_id) {
            // If the key exists, get the corresponding record
            Ok(Some(key)) => {
                let mut record_selector = self
                    .conn
                    .prepare_cached("SELECT modified, data FROM Records WHERE key = ?1")?;
                let mut record_rows = record_selector.query([key])?;

                match record_rows.next() {
                    // Valid record
                    Ok(Some(row)) => Self::cache_response_from_record_row(record_id, row),
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

    fn cache_response_from_record_row(
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

    fn perform_set_cache_transaction(
        tx: &Transaction,
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
        let tx = self.conn.transaction()?;
        Self::perform_set_cache_transaction(&tx, &record)?;
        tx.commit()
    }
}

pub enum CacheResponse {
    Found(Record),
    NotFound(RecordId),
}
