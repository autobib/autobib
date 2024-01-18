use core::convert::AsRef;
use std::path::Path;

use chrono::{DateTime, Local};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Result, Transaction};

use crate::entry::Entry;
use crate::record::*;

pub struct RecordDatabase {
    conn: Connection,
}

impl RecordDatabase {
    /// Execute the database initialization.
    fn initialize_database(tx: &Transaction) -> Result<(), rusqlite::Error> {
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
                 CONSTRAINT foreign_record_key
                      FOREIGN KEY (record_key)
                      REFERENCES Records(key)
                      ON DELETE CASCADE
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

        // Enable foreign keys
        tx.execute("PRAGMA foreign_keys = ON;", ())?;

        Ok(())
    }

    /// Create a new database file and initialize the table structure.
    pub fn create<P: AsRef<Path>>(db_file: P) -> Result<Self, rusqlite::Error> {
        let mut conn = Connection::open(db_file)?;

        let tx = conn.transaction()?;
        Self::initialize_database(&tx)?;
        tx.commit()?;

        Ok(RecordDatabase { conn })
    }

    /// Opening an existing database file on disk.
    pub fn open<P: AsRef<Path>>(db_file: P) -> Result<Self, rusqlite::Error> {
        Ok(RecordDatabase {
            conn: Connection::open_with_flags(
                db_file,
                OpenFlags::SQLITE_OPEN_READ_WRITE
                    | OpenFlags::SQLITE_OPEN_URI
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?,
        })
    }

    /// Obtain cached data for '\<source\>:\<sub_id\>'.
    pub fn get_cached_data(
        &mut self,
        record_id: &RecordId,
    ) -> Result<CacheResponse, rusqlite::Error> {
        let tx = self.conn.transaction()?;
        let cache_response = Self::get_cached_data_transaction(&tx, record_id)?;
        tx.commit()?;

        Ok(cache_response)
    }

    /// Insert a new record into the database.
    pub fn set_cached_data(
        &mut self,
        record_id: &RecordId,
        entry: &Entry,
    ) -> Result<(), rusqlite::Error> {
        let tx = self.conn.transaction()?;
        Self::set_cached_data_transaction(&tx, &record_id, &entry)?;
        tx.commit()
    }

    /// Helper function to wrap the insertion into Records and CitationKeys in a transaction.
    fn set_cached_data_transaction(
        tx: &Transaction,
        record_id: &RecordId,
        entry: &Entry,
    ) -> Result<(), rusqlite::Error> {
        let mut setter = tx.prepare_cached(
            "INSERT OR REPLACE INTO Records (record_id, data, modified) values (?1, ?2, ?3)",
        )?;
        setter.execute((
            record_id.full_id(),
            serde_json::to_string(&entry).unwrap(), // TODO: proper error here
            &Local::now(),
        ))?;

        // get identifier
        let key = tx.last_insert_rowid();

        // add citation keys
        let mut key_writer = tx.prepare_cached(
            "INSERT OR REPLACE INTO CitationKeys (name, record_key) values (?1, ?2)",
        )?;
        key_writer.execute((record_id.full_id(), key))?;

        Ok(())
    }

    pub fn set_cached_null_record(&mut self, record_id: &RecordId) -> Result<(), rusqlite::Error> {
        let tx = self.conn.transaction()?;
        Self::set_cached_null_record_transaction(&tx, record_id)?;
        tx.commit()
    }

    fn get_cached_data_transaction(
        tx: &Transaction,
        record_id: &RecordId,
    ) -> Result<CacheResponse, rusqlite::Error> {
        // First, try to get the key from CitationKeys
        match Self::get_record_key(&tx, &record_id) {
            // If the key exists, get the corresponding record
            Ok(Some(key)) => {
                let mut record_selector =
                    tx.prepare_cached("SELECT modified, data FROM Records WHERE key = ?1")?;
                let mut record_rows = record_selector.query([key])?;

                match record_rows.next() {
                    // Valid record
                    Ok(Some(row)) => Self::cache_response_from_record_row(record_id, row)
                        .map(CacheResponse::Found),
                    Ok(None) => {
                        panic!("A key in CitationKeys does not correspond to a row in Records!")
                    }
                    Err(err) => Err(err),
                }
            }
            // No key, check for cache in NullRecords
            Ok(None) => {
                let mut null_selector =
                    tx.prepare_cached("SELECT attempted FROM NullRecords WHERE record_id = ?1")?;
                let mut null_rows = null_selector.query([&record_id.full_id()])?;

                match null_rows.next() {
                    // Cached null
                    Ok(Some(row)) => Ok(CacheResponse::FoundNull(row.get("attempted")?)),
                    Ok(None) => Ok(CacheResponse::NotFound),
                    Err(err) => Err(err),
                }
            }
            Err(err) => Err(err),
        }
    }

    /// Determine the key for the internal Records table corresponding to '\<source\>:\<sub_id\>'.
    fn get_record_key(
        tx: &Transaction,
        record_id: &RecordId,
    ) -> Result<Option<usize>, rusqlite::Error> {
        let mut selector =
            tx.prepare_cached("SELECT record_key FROM CitationKeys WHERE name = ?1")?;

        selector
            .query_row([record_id.full_id()], |row| row.get("record_key"))
            .optional()
    }

    /// Helper function to wrap the insertion into NullRecords in a transaction.
    fn set_cached_null_record_transaction(
        tx: &Transaction,
        record_id: &RecordId,
    ) -> Result<(), rusqlite::Error> {
        let mut setter = tx.prepare_cached(
            "INSERT OR REPLACE INTO NullRecords (record_id, attempted) values (?1, ?2)",
        )?;
        setter.execute((record_id.full_id(), Local::now()))?;
        Ok(())
    }

    /// Convert a row into a CacheResponse.
    /// This assumes that the row was generated by the following query:
    /// ```sql
    /// SELECT modified, data FROM Records WHERE ...
    /// ```
    fn cache_response_from_record_row(
        record_id: &RecordId,
        row: &rusqlite::Row,
    ) -> Result<Entry, rusqlite::Error> {
        let data_str: String = row.get("data")?;
        let modified: DateTime<Local> = row.get("modified")?;

        Ok(serde_json::from_str(&data_str).unwrap())
    }
}

/// Represent the possible return types of a request for cached data.
/// 1. Found(Record) where Record.data is Some(Entry): the cache exists, and contains data.
/// 2. Found(Record) where Record.data is None: the cache exists, and is null.
/// 3. NotFound(RecordId): RecordId has not been cached.
pub enum CacheResponse {
    Found(Entry),
    FoundNull(DateTime<Local>),
    NotFound,
}
