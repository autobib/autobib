//! # Core database implementation
//! This module implements the abstraction over the underlying [SQLite](https://sqlite.org/)
//! database in which all bibliographic data is stored.
//!
//! The core struct is the [`RecordDatabase`], as well as the data objects [`RecordData`],
//! [`RawRecordData`], and the corresponding trait [`EntryData`].
//!
//! ## Description of the internal binary format
//! We use a custom internal binary format to represent the data associated with each bibTex entry.
//!
//! The first byte is the version.
//! Depending on the version, the format is as follows.
//!
//! ### Version 0
//! The data is stored as a sequence of blocks.
//! ```txt
//! HEADER, TYPE, DATA1, DATA2, ...
//! ```
//! The `HEADER` consists of
//! ```txt
//! version: u8,
//! ```
//! and the `TYPE` consists of
//! ```txt
//! [entry_type_len: u8, entry_type: [u8..]]
//! ```
//! Here, `entry_type_len` is the length of `entry_type`, which has length at most [`u8::MAX`].
//! Then, each block `DATA` is of the form
//! ```txt
//! [key_len: u8, value_len: u16, key: [u8..], value: [u8..]]
//! ```
//! where `key_len` is the length of the first `key` segment, and the `value_len` is
//! the length of the `value` segment. Necessarily, `key` and `value` have lengths at
//! most [`u8::MAX`] and [`u16::MAX`] respectively.
//!
//! `value_len` is encoded in little endian format.
//!
//! The `DATA...` are sorted by `key` and each `key` and `entry_type` must be ASCII lowercase. The
//! `entry_type` can be any valid UTF-8.
//!
//! For example we would serialize
//! ```bib
//! @article{...,
//!   Year = {192},
//!   Title = {The Title},
//! }
//! ```
//! as
//! ```
//! # let mut record_data = RecordData::try_new("article".into()).unwrap();
//! # record_data.try_insert("year".into(), "2023".into()).unwrap();
//! # record_data
//! #     .try_insert("title".into(), "The Title".into())
//! #     .unwrap();
//! # let byte_repr = RawRecordData::from(&record_data).into_byte_repr();
//! let expected = vec![
//!     0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 5, 9, 0, b't', b'i', b't', b'l', b'e',
//!     b'T', b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e', b'a', b'r',
//!     b'2', b'0', b'2', b'3',
//! ];
//! # assert_eq!(expected_byte_repr, byte_repr);
//! ```
mod data;
mod sql;

use std::path::Path;

use chrono::{DateTime, Local};
use log::debug;
use rusqlite::{Connection, OptionalExtension, Transaction};

use self::data::ByteRepr;
pub use self::data::{version, EntryData, RawRecordData, RecordData, DATA_MAX_BYTES};
pub(crate) use self::data::{EntryTypeHeader, KeyHeader, ValueHeader};
use self::sql::*;
use crate::error::DatabaseError;
use crate::record::{Alias, RemoteId};

/// An alias for the internal row ID used by SQLite for the `Records` table. This is the `key`
/// column in the table schema defined in [`init_records`].
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
///    resource de-duplication. The table schema is documented in [`init_records`].
/// 2. `CitationKeys`. This is the table used to store any citation key which is inserted into
///    a table. Since multiple citation keys may refer to the same underlying record, this is
///    simply a lookup table for the corresponding record, and the corresponding rows are
///    automatically deleted when the record is deleted. The table schema is documented in
///    [`init_citation_keys`].
/// 3. `NullRecords`. This is a cache table used to keep track of records which are known to
///    not exist. The table schema is documented in [`init_null_records`].
///
/// For a [`RemoteId`], there are two variants:
///
/// - Canonical: if the corresponding source implementation is a
///   [`Resolver`](`crate::source::Resolver`).
/// - Reference: if the corresponding source implementation is a
///   [`Referrer`](`crate::source::Referrer`).
///
/// This distinction is not currently enforced by types, but it may be in the future.
///
/// The two citation key types, [`Alias`] and [`RemoteId`], with the "Canonical" and "Reference"
/// for [`RemoteId`], are stored according to the following table.
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
    ///
    /// Any tables other than the expected tables are ignored.
    pub fn open<P: AsRef<Path>>(db_file: P) -> Result<Self, DatabaseError> {
        debug!(
            "Initializing new connection to `{}`",
            db_file.as_ref().display()
        );
        let mut conn = Connection::open(db_file)?;
        debug!("Enabling write-ahead log");
        conn.prepare_cached(set_wal())?.query_row((), |_| Ok(()))?;

        let tx = conn.transaction()?;

        Self::initialize_table(&tx, "Records", init_records())?;
        Self::initialize_table(&tx, "CitationKeys", init_citation_keys())?;
        Self::initialize_table(&tx, "NullRecords", init_null_records())?;

        tx.commit()?;

        Ok(RecordDatabase { conn })
    }

    /// Optimize the database.
    ///
    /// This should be called when the database connection is closed, or periodically during
    /// long-running operation.
    ///
    /// See the [SQLite docs](https://www.sqlite.org/pragma.html#pragma_optimize)
    /// for more detail.
    pub fn optimize(&mut self) -> Result<(), DatabaseError> {
        debug!("Optimizing database");
        self.conn.execute(optimize(), ())?;
        Ok(())
    }

    /// Validate the schema of an existing table, or return an appropriate error.
    fn validate_table_schema(
        tx: &Transaction,
        table_name: &str,
        expected_schema: &str,
    ) -> Result<(), DatabaseError> {
        let mut table_selector = tx.prepare_cached(get_table_schema())?;
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
        debug!("Initializing new or validating existing table `{table_name}`");
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
        debug!("Looking up cached data for `{}`", citation_key.name());
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
                let mut record_selector = tx.prepare_cached(get_cached_data())?;
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
    pub fn get_cached_data_and_ref<'a, K: CitationKey, R: Iterator<Item = &'a RemoteId>>(
        &mut self,
        citation_key: &K,
        refs: R,
    ) -> Result<RecordsResponse, DatabaseError> {
        debug!("Getting cached data for `{}`", citation_key.name());
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
                let mut record_selector = tx.prepare_cached(get_cached_data())?;
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
    /// This assumes that the row was generated by the query detailed in [`get_cached_data`].
    fn cache_response_from_record_row(
        row: &rusqlite::Row,
    ) -> Result<(RawRecordData, RemoteId, DateTime<Local>), DatabaseError> {
        let data_blob: Vec<u8> = row.get("data")?;
        let record_id_str: String = row.get("record_id")?;
        let modified: DateTime<Local> = row.get("modified")?;

        Ok((
            // SAFETY: we assume that the underlying database is correctly formatted
            unsafe { RawRecordData::from_byte_repr_unchecked(data_blob) },
            RemoteId::new_unchecked(record_id_str),
            modified,
        ))
    }

    /// Insert a new record into the database.
    ///
    /// Every record requires that it is associated with a canonical [`RemoteId`] with a
    /// corresponding entry. There may also be associated references.
    pub fn set_cached_data<'a, R: Iterator<Item = &'a RemoteId>, D: ByteRepr>(
        &mut self,
        canonical_id: &RemoteId,
        record_data: D,
        remote_id_iter: R,
    ) -> Result<(), DatabaseError> {
        debug!("Setting cached data for `{canonical_id}`");
        let tx = self.conn.transaction()?;
        Self::set_cached_data_tx(&tx, canonical_id, record_data, remote_id_iter)?;
        Ok(tx.commit()?)
    }

    /// Helper function to wrap the insertion into Records and CitationKeys in a transaction.
    fn set_cached_data_tx<'a, R: Iterator<Item = &'a RemoteId>, D: ByteRepr>(
        tx: &Transaction,
        canonical_id: &RemoteId,
        record_data: D,
        remote_id_iter: R,
    ) -> Result<(), DatabaseError> {
        let mut setter = tx.prepare_cached(set_cached_data())?;
        setter.execute((
            canonical_id.name(),
            record_data.into_byte_repr(),
            &Local::now(),
        ))?;

        // get identifier
        let key = tx.last_insert_rowid();
        debug!("Cached data assigned internal ID `{key}`");

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
        debug!("Looking up cached null for `{remote_id}`");
        let tx = self.conn.transaction()?;
        let response = Self::get_cached_null_tx(&tx, remote_id)?;
        tx.commit()?;
        Ok(response)
    }

    /// Check if the [`RemoteId`] is a cached null record within a transaction.
    ///
    /// Here, we allow `target` to be any `CitationKey` since sometimes it is convenient to check for
    /// the presence of an arbitrary `CitationKey` without wanting to first determine if it is a
    /// `RemoteId`.
    fn get_cached_null_tx<K: CitationKey>(
        tx: &Transaction,
        target: &K,
    ) -> Result<NullRecordsResponse, DatabaseError> {
        let mut null_selector = tx.prepare_cached(get_cached_null())?;
        let mut null_rows = null_selector.query([&target.name()])?;

        match null_rows.next() {
            Ok(Some(row)) => Ok(NullRecordsResponse::Found(row.get("attempted")?)),
            Ok(None) => Ok(NullRecordsResponse::NotFound),
            Err(err) => Err(err.into()),
        }
    }

    /// Cache a null record.
    ///
    /// If the [`RemoteId`] is a canonical variant, this means that there is no associated entry. If
    /// [`RemoteId`] is a reference variant, this means there is no associated canonical
    /// [`RemoteId`].
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
        let mut setter = tx.prepare_cached(set_cached_null())?;
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
        let mut updater = tx.prepare_cached(rename_citation_key())?;
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
        let mut deleter = tx.prepare_cached(delete_citation_key())?;
        if deleter.execute((citation_key.name(),))? == 0 {
            Err(DatabaseError::AliasDeleteMissing(
                citation_key.name().into(),
            ))
        } else {
            Ok(())
        }
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

    /// Insert a new citation key referencing a [`DatabaseEntryId`].
    fn set_citation_key_row_tx<K: CitationKey>(
        tx: &Transaction,
        name: &K,
        key: DatabaseEntryId,
        mode: CitationKeyInsertMode,
    ) -> Result<(), DatabaseError> {
        debug!(
            "Creating CitationKey row `{}` for internal ID `{key}`",
            name.name()
        );

        let stmt = match mode {
            CitationKeyInsertMode::Overwrite => set_citation_key_overwrite(),
            CitationKeyInsertMode::FailIfExists => set_citation_key_fail(),
            CitationKeyInsertMode::IgnoreIfExists => set_citation_key_ignore(),
        };
        let mut key_writer = tx.prepare_cached(stmt)?;
        Self::map_citation_key_result(key_writer.execute((name.name(), key)), name)
    }

    /// Determine the [`DatabaseEntryId`] corresponding to [`CitationKey`].
    ///
    /// This is performed within a transaction since typically you want to use the resulting row
    /// identifier for subsequent queries (e.g. to retrieve the corresponding record), in which
    /// case you want to guarantee that the corresponding row still exists.
    fn get_record_key<K: CitationKey>(
        tx: &Transaction,
        record_id: &K,
    ) -> Result<Option<DatabaseEntryId>, DatabaseError> {
        let mut selector = tx.prepare_cached(get_record_key())?;

        Ok(selector
            .query_row([record_id.name()], |row| row.get("record_key"))
            .optional()?)
    }
}

/// Response type from the `Records` table as returned by [`RecordDatabase::get_cached_data`].
pub enum RecordsResponse {
    /// Data was found; canonical; last modified.
    Found(RawRecordData, RemoteId, DateTime<Local>),
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
    /// Overwrite an existing citation key, if any.
    Overwrite,
    /// Fail if there is an existing citation key.
    FailIfExists,
    /// Ignore if there is an existing citation key.
    IgnoreIfExists,
}
