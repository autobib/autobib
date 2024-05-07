use core::convert::AsRef;
use std::path::Path;

use chrono::{DateTime, Local};
use rusqlite::{Connection, OptionalExtension, Result, Transaction};

use crate::entry::Entry;
use crate::record::*;

pub struct RecordDatabase {
    conn: Connection,
}

impl RecordDatabase {
    /// Create or open a database file.
    ///
    /// If the expected tables are missing, create them. If the expected tables already exist but
    /// do not have the expected schema, this causes an error.
    ///
    /// The expected tables are as follows.
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
    pub fn open<P: AsRef<Path>>(db_file: P) -> Result<Self, DatabaseError> {
        // create a new database if it does not exist; otherwise open the existing database
        let mut conn = Connection::open(db_file)?;
        let tx = conn.transaction()?;

        tx.execute("PRAGMA foreign_keys = ON;", ())?;

        // validate the expected table schemas, creating missing tables if they do not exist
        Self::initialize_table(&tx, "Records", include_str!("database/records.sql"))?;
        Self::initialize_table(
            &tx,
            "CitationKeys",
            include_str!("database/citation_keys.sql"),
        )?;
        Self::initialize_table(
            &tx,
            "NullRecords",
            include_str!("database/null_records.sql"),
        )?;

        tx.commit()?;

        Ok(RecordDatabase { conn })
    }

    /// Validate the table schema of an existing table, or return an appropriate error.
    fn validate_table_schema(
        tx: &Transaction,
        table_name: &str,
        expected_schema: &str,
    ) -> Result<(), DatabaseError> {
        let mut table_selector =
            tx.prepare_cached("SELECT sql FROM sqlite_schema WHERE name = ?1;")?;
        let mut record_rows = table_selector.query([table_name])?;
        match record_rows.next() {
            Ok(Some(row)) => {
                let table_schema: String = row.get("sql")?;
                if table_schema == expected_schema {
                    Ok(())
                } else {
                    Err(DatabaseError::TableIncorrectSchema(
                        table_name.into(),
                        table_schema,
                    ))
                }
            }
            Ok(None) => Err(DatabaseError::TableMissing(table_name.into())),
            Err(why) => Err(why.into()),
        }
    }

    /// Initialize a table inside a transaction.
    fn initialize_table(
        tx: &Transaction,
        table_name: &str,
        schema: &str,
    ) -> Result<(), DatabaseError> {
        match Self::validate_table_schema(&tx, table_name, schema) {
            Ok(()) => Ok(()),
            Err(DatabaseError::TableMissing(_)) => {
                tx.execute(schema, ())?;
                Ok(())
            }
            Err(e) => Err(e),
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
                        let mut null_rows = null_selector.query([&record_id.name()])?;

                        match null_rows.next() {
                            // Cached null
                            Ok(Some(row)) => Ok(CacheResponse::FoundNull(row.get("attempted")?)),
                            Ok(None) => Ok(CacheResponse::NotFound(record_id)),
                            Err(err) => Err(err.into()),
                        }
                    }
                    // If it is an Alias, the CitationKeys table is the canonical source for
                    // whether or not the alias is set.
                    CitationKeyInput::Alias(alias) => {
                        Ok(CacheResponse::NullAlias(alias.name().into()))
                    }
                }
            }
            Err(err) => Err(err),
        }
    }

    /// Rename an alias.
    pub fn rename_alias(&mut self, alias: &Alias, new: &Alias) -> Result<(), DatabaseError> {
        let tx = self.conn.transaction()?;
        let status = Self::rename_alias_transaction(&tx, alias, new)?;
        tx.commit()?;
        Ok(status)
    }

    /// Rename an alias within a transaction.
    fn rename_alias_transaction(
        tx: &Transaction,
        name: &Alias,
        new: &Alias,
    ) -> Result<(), DatabaseError> {
        let mut updater = tx.prepare_cached("UPDATE CitationKeys SET name = ?1 WHERE name = ?2")?;
        Self::map_citation_key_result(updater.execute((new.name(), name.name())), name)
    }

    /// Take the result of a SQLite operation, suppressing the output and processing the error.
    fn map_citation_key_result<R, T: CitationKey>(
        res: Result<R, rusqlite::Error>,
        citation_key: &T,
    ) -> Result<(), DatabaseError> {
        match res {
            Ok(_) => Ok(()),
            Err(err) => match err.sqlite_error_code() {
                // the UNIQUE constraint is violated, so the key already exists
                Some(rusqlite::ErrorCode::ConstraintViolation) => {
                    Err(DatabaseError::CitationKeyExists(citation_key.name().into()))
                }
                _ => Err(err.into()),
            },
        }
    }

    /// Delete an alias.
    pub fn delete_alias(&mut self, alias: &Alias) -> Result<(), DatabaseError> {
        let tx = self.conn.transaction()?;
        let status = Self::delete_citation_key_row_transaction(&tx, alias)?;
        tx.commit()?;
        Ok(status)
    }

    /// Delete a citation key row within a transaction.
    fn delete_citation_key_row_transaction<T: CitationKey>(
        tx: &Transaction,
        name: &T,
    ) -> Result<(), DatabaseError> {
        let mut deleter = tx.prepare_cached("DELETE FROM CitationKeys WHERE name = ?1")?;
        Ok(deleter.execute((name.name(),)).map(|_| ())?)
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
            // TODO: provide a better error message if citation key is missing and
            //       the corresponding citation key is in NullRecords
            Ok(None) => Err(DatabaseError::CitationKeyMissing(String::from(
                target.name(),
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
        Self::map_citation_key_result(key_writer.execute((name.name(), key)), name)
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
            .query_row([citation_key.name()], |row| row.get("record_key"))
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
    TableMissing(String),
    TableIncorrectSchema(String, String),
    CitationKeyExists(String),
    CitationKeyMissing(String),
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseError::SQL(err) => err.fmt(f),
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
    NullAlias(String),
    /// The search was for a [`RecordId`], which did not exist.
    NotFound(&'a RecordId),
}
