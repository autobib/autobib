use core::convert::AsRef;
use std::path::Path;

use chrono::{DateTime, Local};
use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::entry::Entry;
use crate::error::DatabaseError;
use crate::record::{Alias, RemoteId};

type DatabaseEntryId = i64;

/// This trait represents types which can be stored as a row in the SQL database underlying a
/// [`RecordDatabase`].
pub trait CitationKey {
    /// The string to use as the key for a row.
    fn name(&self) -> &str;
}

/// Internal representation of the underlying SQL database.
///
/// The table structure is as follows.
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
/// The two citation key types, [`Alias`] and [`RemoteId`], with the variants `CanonicalId` and
/// `ReferenceId` for [`RemoteId`], are stored according to the following table.
///
/// |            | Stored in Records | Stored in NullRecords | Stored in CitationKeys |
/// |------------|-------------------|-----------------------|------------------------|
/// |CanonicalId |        YES        |          YES          |          YES           |
/// |ReferenceId |        NO         |          YES          |          YES           |
/// |Alias       |        NO         |          NO           |          YES           |
pub struct RecordDatabase {
    conn: Connection,
}

impl RecordDatabase {
    /// Open a database file.
    ///
    /// If the expected tables are missing, create them. If the expected tables already exist but
    /// do not have the expected schema, this causes an error. The expected tables are as
    /// detailed in the documentation for [`RecordDatabase`].
    pub fn open<P: AsRef<Path>>(db_file: P) -> Result<Self, DatabaseError> {
        // create a new database if it does not exist; otherwise open the existing database
        let mut conn = Connection::open(db_file)?;
        let tx = conn.transaction()?;

        // initialize connection state; e.g. set foreign_keys = ON
        tx.execute(include_str!("database/initialize.sql"), ())?;

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

    /// Optimize the database.
    ///
    /// This should be called when the database connection is closed, or periodically during
    /// long-running operation.
    ///
    /// See [SQLite docs](https://www.sqlite.org/pragma.html#pragma_optimize) for more detail.
    pub fn optimize(&mut self) -> Result<(), DatabaseError> {
        self.conn.execute("PRAGMA optimize", ())?;
        Ok(())
    }

    /// Validate the schema of an existing table, or return an appropriate error.
    fn validate_table_schema(
        tx: &Transaction,
        table_name: &str,
        expected_schema: &str,
    ) -> Result<(), DatabaseError> {
        let mut table_selector =
            tx.prepare_cached(include_str!("database/get_table_schema.sql"))?;
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
        match Self::validate_table_schema(tx, table_name, schema) {
            Ok(()) => Ok(()),
            Err(DatabaseError::TableMissing(_)) => {
                tx.execute(schema, ())?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Get the cached data corresponding to a [`CitationKey`].
    pub fn get_cached_data<K: CitationKey>(
        &mut self,
        citation_key: &K,
    ) -> Result<RecordsResponse, DatabaseError> {
        let tx = self.conn.transaction()?;
        let response = Self::get_cached_data_tx(&tx, citation_key)?;
        tx.commit()?;
        Ok(response)
    }

    /// Get the cached data corresponding to a [`CitationKey`] inside a transaction.
    fn get_cached_data_tx<K: CitationKey>(
        tx: &Transaction,
        citation_key: &K,
    ) -> Result<RecordsResponse, DatabaseError> {
        match Self::get_record_key(tx, citation_key)? {
            // target exists
            Some(key) => {
                let mut record_selector =
                    tx.prepare_cached(include_str!("database/get_cached_data.sql"))?;
                let mut record_rows = record_selector.query([key])?;

                // SAFETY: key always corresponds to a valid row
                //         because of ON DELETE CASCADE
                let row = record_rows.next()?.expect("RowId does not exist!)");
                Self::cache_response_from_record_row(row).map(|(entry, canonical, modified)| {
                    RecordsResponse::Found(entry, canonical, modified)
                })
            }
            None => Ok(RecordsResponse::NotFound),
        }
    }

    /// Get the cached data corresponding to a [`CitationKey`].
    // pub fn get_cached_data_and_ref<K: CitationKey, R: Iterator>(
    pub fn get_cached_data_and_ref<'a, K: CitationKey, R: Iterator<Item = &'a RemoteId>>(
        &mut self,
        citation_key: &K,
        refs: R,
    ) -> Result<RecordsResponse, DatabaseError> {
        let tx = self.conn.transaction()?;
        let response = Self::get_cached_data_and_ref_tx(&tx, citation_key, refs)?;
        tx.commit()?;
        Ok(response)
    }

    /// Get the cached data corresponding to a [`CitationKey`] inside a transaction.
    fn get_cached_data_and_ref_tx<'a, K: CitationKey, R: Iterator<Item = &'a RemoteId>>(
        tx: &Transaction,
        citation_key: &K,
        refs: R,
    ) -> Result<RecordsResponse, DatabaseError> {
        match Self::get_record_key(tx, citation_key)? {
            // target exists
            Some(key) => {
                let mut record_selector =
                    tx.prepare_cached(include_str!("database/get_cached_data.sql"))?;
                let mut record_rows = record_selector.query([key])?;

                // insert refs
                for remote_id in refs {
                    Self::set_citation_key_row_tx(
                        tx,
                        remote_id,
                        key,
                        CitationKeyInsertMode::Overwrite,
                    )?;
                }

                // SAFETY: key always corresponds to a valid row
                //         because of ON DELETE CASCADE
                let row = record_rows.next()?.expect("RowId does not exist!)");
                Self::cache_response_from_record_row(row).map(|(entry, canonical, modified)| {
                    RecordsResponse::Found(entry, canonical, modified)
                })
            }
            None => Ok(RecordsResponse::NotFound),
        }
    }

    /// Process a [`rusqlite::Row`] into a manageable type.
    ///
    /// This assumes that the row was generated by the following query:
    /// ```sql
    /// SELECT record_id, modified, data FROM Records WHERE ...
    /// ```
    fn cache_response_from_record_row(
        row: &rusqlite::Row,
    ) -> Result<(Entry, RemoteId, DateTime<Local>), DatabaseError> {
        let data_str: String = row.get("data")?;
        let record_id_str: String = row.get("record_id")?;
        let modified: DateTime<Local> = row.get("modified")?;

        Ok((
            // TODO: fixme when `set_cached_data` is fixed
            serde_json::from_str(&data_str).unwrap(),
            RemoteId::new_unchecked(record_id_str),
            modified,
        ))
    }

    /// Insert a new record into the database.
    ///
    /// Every record requires that it is associated with a canonical [`RemoteId`] with a
    /// corresponding entry. There may also be associated references.
    pub fn set_cached_data<'a, R: Iterator<Item = &'a RemoteId>>(
        &mut self,
        canonical_id: &RemoteId,
        entry: &Entry,
        remote_id_iter: R,
    ) -> Result<(), DatabaseError> {
        let tx = self.conn.transaction()?;
        Self::set_cached_data_tx(&tx, canonical_id, entry, remote_id_iter)?;
        Ok(tx.commit()?)
    }

    /// Helper function to wrap the insertion into Records and CitationKeys in a transaction.
    fn set_cached_data_tx<'a, R: Iterator<Item = &'a RemoteId>>(
        tx: &Transaction,
        canonical_id: &RemoteId,
        entry: &Entry,
        remote_id_iter: R,
    ) -> Result<(), DatabaseError> {
        let mut setter = tx.prepare_cached(include_str!("database/set_cached_data.sql"))?;
        setter.execute((
            canonical_id.name(),
            // TODO: do something more sensible, and fix `cache_response_from_record_row`
            serde_json::to_string(&entry).unwrap(),
            &Local::now(),
        ))?;

        // get identifier
        let key = tx.last_insert_rowid();

        // add citation keys
        for remote_id in remote_id_iter {
            Self::set_citation_key_row_tx(tx, remote_id, key, CitationKeyInsertMode::Overwrite)?;
        }

        Ok(())
    }

    /// Check if the [`RemoteId`] is a cached null record.
    pub fn get_cached_null(
        &mut self,
        remote_id: &RemoteId,
    ) -> Result<NullRecordsResponse, DatabaseError> {
        let tx = self.conn.transaction()?;
        let response = Self::get_cached_null_tx(&tx, remote_id)?;
        tx.commit()?;
        Ok(response)
    }

    /// Check if the [`RemoteId`] is a cached null record within a transaction.
    ///
    /// We allow `target` to be any `CitationKey` since sometimes it is convenient to check for the
    /// presence of an arbitrary `CitationKey` without wanting to first determine if it is a
    /// `RemoteId`.
    fn get_cached_null_tx<K: CitationKey>(
        tx: &Transaction,
        target: &K,
    ) -> Result<NullRecordsResponse, DatabaseError> {
        let mut null_selector = tx.prepare_cached(include_str!("database/get_cached_null.sql"))?;
        let mut null_rows = null_selector.query([&target.name()])?;

        match null_rows.next() {
            // Cached null
            Ok(Some(row)) => Ok(NullRecordsResponse::Found(row.get("attempted")?)),
            Ok(None) => Ok(NullRecordsResponse::NotFound),
            Err(err) => Err(err.into()),
        }
    }

    /// Cache a null record.
    ///
    /// If `record_id` has a canonical source, this means that there is no associated entry. If
    /// `record_id` is a reference source, this means there is no associated canonical `record_id`.
    pub fn set_cached_null<'a, R: Iterator<Item = &'a RemoteId>>(
        &mut self,
        remote_id_iter: R,
    ) -> Result<(), DatabaseError> {
        let tx = self.conn.transaction()?;
        Self::set_cached_null_tx(&tx, remote_id_iter)?;
        Ok(tx.commit()?)
    }

    /// Helper function to wrap the insertion into NullRecords in a transaction.
    fn set_cached_null_tx<'a, R: Iterator<Item = &'a RemoteId>>(
        tx: &Transaction,
        remote_id_iter: R,
    ) -> Result<(), DatabaseError> {
        let mut setter = tx.prepare_cached(include_str!("database/set_cached_null.sql"))?;
        let cache_time = Local::now();
        for remote_id in remote_id_iter {
            setter.execute((remote_id.name(), cache_time))?;
        }

        Ok(())
    }

    /// Rename an alias.
    pub fn rename_alias(&mut self, old: &Alias, new: &Alias) -> Result<(), DatabaseError> {
        let tx = self.conn.transaction()?;
        Self::rename_alias_tx(&tx, old, new)?;
        tx.commit()?;
        Ok(())
    }

    /// Rename an alias within a transaction.
    fn rename_alias_tx(tx: &Transaction, old: &Alias, new: &Alias) -> Result<(), DatabaseError> {
        let mut updater = tx.prepare_cached(include_str!("database/rename_citation_key.sql"))?;
        Self::map_citation_key_result(updater.execute((new.name(), old.name())), old)
    }

    /// Take the result of a SQLite operation, suppressing the output and processing the error.
    fn map_citation_key_result<T, K: CitationKey>(
        res: Result<T, rusqlite::Error>,
        citation_key: &K,
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
        Self::delete_citation_key_row_tx(&tx, alias)?;
        tx.commit()?;
        Ok(())
    }

    /// Delete a citation key row within a transaction.
    fn delete_citation_key_row_tx<K: CitationKey>(
        tx: &Transaction,
        citation_key: &K,
    ) -> Result<(), DatabaseError> {
        let mut deleter =
            tx.prepare_cached("DELETE FROM CitationKeys WHERE name = ?1 RETURNING *")?;
        deleter
            .query_row([citation_key.name()], |_| Ok(()))
            .optional()?
            .map_or_else(
                || {
                    Err(DatabaseError::AliasDeleteMissing(
                        citation_key.name().into(),
                    ))
                },
                |_| Ok(()),
            )
    }

    /// Insert a new citation alias.
    pub fn insert_alias<K: CitationKey>(
        &mut self,
        alias: &Alias,
        target: &K,
    ) -> Result<(), DatabaseError> {
        let tx = self.conn.transaction()?;
        Self::insert_alias_tx(&tx, alias, target)?;
        tx.commit()?;
        Ok(())
    }

    /// Insert a new citation alias within a transaction.
    fn insert_alias_tx<K: CitationKey>(
        tx: &Transaction,
        alias: &Alias,
        target: &K,
    ) -> Result<(), DatabaseError> {
        match Self::get_record_key(tx, target) {
            // target exists
            Ok(Some(key)) => {
                Self::set_citation_key_row_tx(tx, alias, key, CitationKeyInsertMode::FailIfExists)
            }
            // target does not exist
            Ok(None) => match Self::get_cached_null_tx(tx, target)? {
                NullRecordsResponse::Found(_) => {
                    Err(DatabaseError::CitationKeyNull(target.name().into()))
                }
                NullRecordsResponse::NotFound => {
                    Err(DatabaseError::CitationKeyMissing(target.name().into()))
                }
            },
            Err(why) => Err(why),
        }
    }

    /// Insert a new citation key referencing the internal key `key`.
    fn set_citation_key_row_tx<K: CitationKey>(
        tx: &Transaction,
        name: &K,
        key: DatabaseEntryId,
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

    /// Determine the key for the internal Records table corresponding to [`CitationKey`].
    ///
    /// This is performed within a transaction since typically you want to use the resulting row
    /// identifier for subsequent queries (e.g. to retrieve the corresponding record), in which
    /// case you want to guarantee that the corresponding row still exists.
    fn get_record_key<K: CitationKey>(
        tx: &Transaction,
        record_id: &K,
    ) -> Result<Option<DatabaseEntryId>, DatabaseError> {
        let mut selector = tx.prepare_cached(include_str!("database/get_record_key.sql"))?;

        Ok(selector
            .query_row([record_id.name()], |row| row.get("record_key"))
            .optional()?)
    }
}

/// Response type from the `Records` table as returned by [`RecordDatabase::get_cached_data`].
#[allow(clippy::large_enum_variant)]
pub enum RecordsResponse {
    /// Data was found; canonical; last modified.
    Found(Entry, RemoteId, DateTime<Local>),
    /// Data was not found.
    NotFound,
}

/// Response type from the `NullRecords` table as returned by [`RecordDatabase::get_cached_null`].
pub enum NullRecordsResponse {
    /// Null was found; last attempted.
    Found(DateTime<Local>),
    /// Null was not found.
    NotFound,
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
