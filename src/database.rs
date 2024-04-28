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
    /// Create a new database file and initialize the table structure.
    ///
    /// The tables are as follows.
    ///
    /// 1. `Records`. This is the primary table used to store records. The integer primary key
    ///    `key` is used as the internal unambiguous reference for each record and is used for
    ///    resource de-duplication.
    /// 2. `CitationKeys`. This is the table used to store any citation key which is inserted into
    ///    a table. Since multiple citation keys may refer to the same underlying record, this is
    ///    simply a lookup table for the corresponding record, and the corresponding rows are
    ///    automatically deleted when the record is deleted.
    /// 3. `NullRecords`. This is a cache table used to keep track of records which are known to
    ///    not exist.
    ///
    /// The two citation key types, [`Alias`] and [`RecordId`], with the variants `CanonicalId` and
    /// `ReferenceId` for [`RecordId`], are stored according to the following table.
    ///
    ///             | Stored in Records | Stored in NullRecords | Stored in CitationKeys
    /// ------------|-------------------|-----------------------|-----------------------
    /// CanonicalId |        YES        |          YES          |          YES
    /// ReferenceId |        NO         |          YES          |          YES
    /// Alias       |        NO         |          NO           |          YES
    pub fn create<P: AsRef<Path>>(db_file: P) -> Result<Self, DatabaseError> {
        let mut conn = Connection::open(db_file)?;

        let tx = conn.transaction()?;
        Self::initialize_database(&tx)?;
        tx.commit()?;

        Ok(RecordDatabase { conn })
    }

    /// Initialize the tables within a transaction.
    fn initialize_database(tx: &Transaction) -> Result<(), DatabaseError> {
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

    /// Open an existing database file on disk.
    pub fn open<P: AsRef<Path>>(db_file: P) -> Result<Self, DatabaseError> {
        Ok(RecordDatabase {
            // TODO: handle invalid schema
            conn: Connection::open_with_flags(
                db_file,
                OpenFlags::SQLITE_OPEN_READ_WRITE
                    | OpenFlags::SQLITE_OPEN_URI
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?,
        })
    }

    /// Open an existing database file, or create it if it does not exist.
    pub fn open_or_create<P: AsRef<Path>>(db_file: P) -> Result<Self, DatabaseError> {
        match Self::open(&db_file) {
            Ok(db) => Ok(db),
            // TODO: check if the error is due to the file not existing
            Err(_) => Self::create(&db_file),
        }
    }

    /// Obtain cached data corresponding to a [`CitationKeyInput`].
    ///
    /// Note that the `citation_key` argument must be an actual [`CitationKeyInput`], rather than a
    /// type which implements [`CitationKey`]. The reason for this is that this function must
    /// distinguish between being [`RecordId`] or an [`Alias`], since non-presence in the table in
    /// each case is handled in a slightly different way.
    pub fn get_cached_data<'a>(
        &mut self,
        citation_key: &'a CitationKeyInput,
    ) -> Result<CacheResponse<'a>, DatabaseError> {
        let tx = self.conn.transaction()?;
        let cache_response = Self::get_cached_data_transaction(&tx, citation_key)?;
        tx.commit()?;

        Ok(cache_response)
    }

    /// Obtain cached data corresponding to a [`CitationKeyInput`] within a provided transaction.
    fn get_cached_data_transaction<'a>(
        tx: &Transaction,
        citation_key: &'a CitationKeyInput,
    ) -> Result<CacheResponse<'a>, DatabaseError> {
        // try to get the key from CitationKeys
        match Self::get_record_key(&tx, citation_key) {
            // key exists, get the corresponding record
            Ok(Some(key)) => {
                let mut record_selector =
                    tx.prepare_cached("SELECT modified, data FROM Records WHERE key = ?1")?;
                let mut record_rows = record_selector.query([key])?;

                match record_rows.next() {
                    // Valid record
                    Ok(Some(row)) => Self::cache_response_from_record_row(row)
                        .map(|(entry, modified)| CacheResponse::Found(entry, modified)),
                    Ok(None) => {
                        // SAFETY: the ON DELETE CASCADE and transaction wrapping should prevent
                        // this from ever occurring.
                        panic!("A key in CitationKeys does not correspond to a row in Records!")
                    }
                    Err(err) => Err(err.into()),
                }
            }
            // no key
            Ok(None) => {
                match citation_key {
                    // If CitationKey is a RecordId, check for a cached null record.
                    CitationKeyInput::RecordId(record_id) => {
                        let mut null_selector = tx.prepare_cached(
                            "SELECT attempted FROM NullRecords WHERE record_id = ?1",
                        )?;
                        let mut null_rows = null_selector.query([&record_id.repr()])?;

                        match null_rows.next() {
                            // Cached null
                            Ok(Some(row)) => Ok(CacheResponse::FoundNull(row.get("attempted")?)),
                            Ok(None) => Ok(CacheResponse::NotFound(record_id)),
                            Err(err) => Err(err.into()),
                        }
                    }
                    // If it is an Alias, the CitationKeys table is the canonical source for
                    // whether or not the alias is set.
                    CitationKeyInput::Alias(_) => Ok(CacheResponse::NullAlias),
                }
            }
            Err(err) => Err(err),
        }
    }

    /// Insert a new citation alias.
    pub fn insert_alias<T: CitationKey>(
        &mut self,
        alias: &Alias,
        target: &T,
    ) -> Result<(), DatabaseError> {
        let tx = self.conn.transaction()?;
        let status = Self::insert_alias_transaction(&tx, alias, target)?;
        tx.commit()?;
        Ok(status)
    }

    /// Insert a new citation alias within a transaction.
    fn insert_alias_transaction<T: CitationKey>(
        tx: &Transaction,
        alias: &Alias,
        target: &T,
    ) -> Result<(), DatabaseError> {
        match Self::get_record_key(tx, target) {
            // target exists
            Ok(Some(key)) => Self::create_citation_key_row_transaction(
                tx,
                alias,
                key,
                CitationKeyInsertMode::FailIfExists,
            ),
            // target does not exist
            Ok(None) => Err(DatabaseError::CitationKeyMissing(String::from(
                target.repr(),
            ))),
            Err(why) => Err(why.into()),
        }
    }

    /// Insert a new citation key referencing the internal key `key`.
    fn create_citation_key_row_transaction<T: CitationKey>(
        tx: &Transaction,
        name: &T,
        key: i64,
        mode: CitationKeyInsertMode,
    ) -> Result<(), DatabaseError> {
        let stmt = match mode {
            CitationKeyInsertMode::Overwrite => {
                "INSERT OR REPLACE INTO CitationKeys (name, record_key) values (?1, ?2)"
            }
            CitationKeyInsertMode::FailIfExists => {
                "INSERT INTO CitationKeys (name, record_key) values (?1, ?2)"
            }
            CitationKeyInsertMode::IgnoreIfExists => {
                "INSERT OR IGNORE INTO CitationKeys (name, record_key) values (?1, ?2)"
            }
        };
        let mut key_writer = tx.prepare_cached(stmt)?;
        match key_writer.execute((name.repr(), key)) {
            Ok(_) => Ok(()),
            Err(err) => match err.sqlite_error_code() {
                // the UNIQUE constraint is violated, so the key already exists
                Some(rusqlite::ErrorCode::ConstraintViolation) => {
                    Err(DatabaseError::CitationKeyExists(name.repr().into()))
                }
                _ => Err(err.into()),
            },
        }
    }

    /// Insert a new record into the database.
    ///
    /// Every record requires that it is associated with a canonical [`RecordId`] with a
    /// corresponding entry. There may also be an associated reference [`RecordId`].
    pub fn set_cached_data(
        &mut self,
        canonical_id: &RecordId,
        entry: &Entry,
        reference_id: Option<&RecordId>,
    ) -> Result<(), DatabaseError> {
        let tx = self.conn.transaction()?;
        Self::set_cached_data_transaction(&tx, &canonical_id, &entry, reference_id)?;
        Ok(tx.commit()?)
    }

    /// Helper function to wrap the insertion into Records and CitationKeys in a transaction.
    fn set_cached_data_transaction(
        tx: &Transaction,
        canonical_id: &RecordId,
        entry: &Entry,
        reference_id: Option<&RecordId>,
    ) -> Result<(), DatabaseError> {
        let mut setter = tx.prepare_cached(
            "INSERT OR REPLACE INTO Records (record_id, data, modified) values (?1, ?2, ?3)",
        )?;
        setter.execute((
            canonical_id.full_id(),
            serde_json::to_string(&entry).unwrap(), // TODO: do something more sensible
            &Local::now(),
        ))?;

        // get identifier
        let key = tx.last_insert_rowid();

        // add citation keys
        Self::create_citation_key_row_transaction(
            tx,
            canonical_id,
            key,
            CitationKeyInsertMode::Overwrite,
        )?;
        if let Some(record_id) = reference_id {
            Self::create_citation_key_row_transaction(
                tx,
                record_id,
                key,
                CitationKeyInsertMode::Overwrite,
            )?;
        }

        Ok(())
    }

    /// Cache a null record.
    ///
    /// If `record_id` has a canonical source, this means that there is no associated entry. If
    /// `record_id` is a reference source, this means there is no associated canonical `record_id`.
    pub fn set_cached_null_record(&mut self, record_id: &RecordId) -> Result<(), DatabaseError> {
        let tx = self.conn.transaction()?;
        Self::set_cached_null_record_transaction(&tx, record_id)?;
        Ok(tx.commit()?)
    }

    /// Helper function to wrap the insertion into NullRecords in a transaction.
    fn set_cached_null_record_transaction(
        tx: &Transaction,
        record_id: &RecordId,
    ) -> Result<(), DatabaseError> {
        let mut setter = tx.prepare_cached(
            "INSERT OR REPLACE INTO NullRecords (record_id, attempted) values (?1, ?2)",
        )?;
        let cache_time = Local::now();
        setter.execute((record_id.full_id(), cache_time))?;

        Ok(())
    }

    /// Determine the key for the internal Records table corresponding to [`CitationKey`].
    ///
    /// This is performed within a transaction since typically you want to use the resulting row
    /// identifier for subsequent queries (e.g. to retrieve the corresponding record), in which
    /// case you want to guarantee that the corresponding row still exists.
    fn get_record_key<T: CitationKey>(
        tx: &Transaction,
        citation_key: &T,
    ) -> Result<Option<i64>, DatabaseError> {
        let mut selector =
            tx.prepare_cached("SELECT record_key FROM CitationKeys WHERE name = ?1")?;

        Ok(selector
            .query_row([citation_key.repr()], |row| row.get("record_key"))
            .optional()?)
    }

    /// Convert a [`rusqlite::Row`] into a [`CacheResponse`].
    ///
    /// This assumes that the row was generated by the following query:
    /// ```sql
    /// SELECT modified, data FROM Records WHERE ...
    /// ```
    fn cache_response_from_record_row(
        row: &rusqlite::Row,
    ) -> Result<(Entry, DateTime<Local>), DatabaseError> {
        let data_str: String = row.get("data")?;
        let modified: DateTime<Local> = row.get("modified")?;

        Ok((serde_json::from_str(&data_str).unwrap(), modified))
    }
}

#[derive(Debug)]
pub enum DatabaseError {
    SQL(rusqlite::Error),
    CitationKeyExists(String),
    CitationKeyMissing(String),
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseError::SQL(err) => err.fmt(f),
            DatabaseError::CitationKeyExists(k) => write!(f, "Citation key exists: '{k}'"),
            DatabaseError::CitationKeyMissing(k) => write!(f, "Citation key missing: '{k}'"),
        }
    }
}

impl From<rusqlite::Error> for DatabaseError {
    fn from(err: rusqlite::Error) -> Self {
        Self::SQL(err)
    }
}

/// The type of citation key insertion to perform.
pub enum CitationKeyInsertMode {
    /// Delete an existing citation key.
    Overwrite,
    /// Fail if there is an existing citation key.
    FailIfExists,
    /// Ignore if there is an existing citation key.
    IgnoreIfExists,
}

/// The responses from the database in a request for cached data.
pub enum CacheResponse<'a> {
    /// Found an [`Entry`], which was last modified at the given time.
    Found(Entry, DateTime<Local>),
    /// The record is null, and this was last checked at the given time.
    FoundNull(DateTime<Local>),
    /// The search was for an alias, which did not exist.
    NullAlias,
    /// The search was for a [`RecordId`], which did not exist.
    NotFound(&'a RecordId),
}
