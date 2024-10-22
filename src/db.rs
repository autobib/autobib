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
pub mod state;
mod validate;

use std::path::Path;

use chrono::{DateTime, Local};
use log::debug;
use nucleo_picker::nucleo::{Injector, Utf32String};
use rusqlite::{types::ValueRef, Connection, OptionalExtension, Transaction};

pub use self::data::{binary_format_version, EntryData, RawRecordData, RecordData, DATA_MAX_BYTES};
pub(crate) use self::data::{EntryTypeHeader, KeyHeader, ValueHeader};
use self::state::{RecordIdState, RemoteIdState};
use self::validate::DatabaseValidator;
use crate::{
    error::{DatabaseError, ValidationError},
    Alias, RecordId, RemoteId,
};

/// The current version of the database table schema.
pub const fn schema_version() -> u8 {
    0
}

/// An alias for the internal row ID used by SQLite for the `Records` and the `NullRecords` table. This is
/// the `key` column in the table schema defined in [`init_records`](sql::init_records), and the
/// implicit `rowid` column in the table schema defined in [`init_null_records`](sql::init_null_records)
type RowId = i64;

/// Determine the [`RowId`] in the `Records` table corresponding to a [`CitationKey`].
fn get_row_id<K: CitationKey>(
    tx: &Transaction,
    record_id: &K,
) -> Result<Option<RowId>, rusqlite::Error> {
    tx.prepare_cached(sql::get_record_key())?
        .query_row([record_id.name()], |row| row.get("record_key"))
        .optional()
}

/// Determine the [`RowId`] in the `NullRecords` table corresponding to a [`CitationKey`].
pub fn get_null_row_id(
    tx: &Transaction,
    remote_id: &RemoteId,
) -> Result<Option<RowId>, rusqlite::Error> {
    tx.prepare_cached(sql::get_null_record_key())?
        .query_row([remote_id.name()], |row| row.get("rowid"))
        .optional()
}

/// The contents of a row in the `Records` table.
pub struct NullRowData {
    /// The last time the row was modified.
    pub attempted: DateTime<Local>,
}

impl TryFrom<&rusqlite::Row<'_>> for NullRowData {
    type Error = rusqlite::Error;

    fn try_from(row: &rusqlite::Row<'_>) -> Result<Self, Self::Error> {
        Ok(Self {
            attempted: row.get("attempted")?,
        })
    }
}

/// The contents of a row in the `Records` table.
pub struct RowData {
    /// The binary data associated with the row.
    pub data: RawRecordData,
    /// The canonical record id associated with the row.
    pub canonical: RemoteId,
    /// The last time the row was modified.
    pub modified: DateTime<Local>,
}

impl TryFrom<&rusqlite::Row<'_>> for RowData {
    type Error = rusqlite::Error;

    fn try_from(row: &rusqlite::Row<'_>) -> Result<Self, Self::Error> {
        Ok(Self {
            // SAFETY: we assume that the underlying database is correctly formatted
            data: unsafe { RawRecordData::from_byte_repr_unchecked(row.get("data")?) },
            canonical: unsafe { RemoteId::from_string_unchecked(row.get("record_id")?) },
            modified: row.get("modified")?,
        })
    }
}

/// This trait represents types which can be stored as a row in the SQL database underlying a
/// [`RecordDatabase`].
pub trait CitationKey: private::Sealed {
    /// The string to use as the key for a row.
    fn name(&self) -> &str;
}

mod private {
    /// Prevent implemntation of [`CitationKey`](super::CitationKey) by foreign types.
    pub trait Sealed {}

    impl Sealed for crate::Alias {}
    impl Sealed for crate::RecordId {}
    impl Sealed for crate::RemoteId {}
}

/// Internal representation of the underlying SQL database.
///
/// The table structure is as follows.
///
/// 1. `Records`. This is the primary table used to store records. The integer primary key
///    `key` is used as the internal unambiguous reference for each record and is used for
///    de-duplication. The table schema is documented in [`init_records`](sql::init_records).
/// 2. `CitationKeys`. This is the table used to store any citation key which is inserted into
///    a table. Since multiple citation keys may refer to the same underlying record, this is
///    simply a lookup table for the corresponding record, and the corresponding rows are
///    automatically deleted when the record is deleted. The table schema is documented in
///    [`init_citation_keys`](sql::init_citation_keys).
/// 3. `NullRecords`. This is a cache table used to keep track of records which are known to
///    not exist. The table schema is documented in [`init_null_records`](sql::init_null_records).
///
/// For a [`RemoteId`], there are two variants:
///
/// - Canonical: if the corresponding provider implementation is a
///   [`Resolver`](`crate::provider::Resolver`).
/// - Reference: if the corresponding provider implementation is a
///   [`Referrer`](`crate::provider::Referrer`).
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
    /// Open a database file at the provided [`Path`].
    ///
    /// If the expected tables are missing, create them. If the expected tables already exist but
    /// do not have the expected schema, this results in an error. The expected table schemas are
    /// detailed in the documentation for [`RecordDatabase`].
    ///
    /// Any tables other than the expected tables are ignored.
    pub fn open<P: AsRef<Path>>(db_path: P) -> Result<Self, DatabaseError> {
        debug!(
            "Initializing new connection to '{}'",
            db_path.as_ref().display()
        );
        let mut conn = Connection::open(db_path)?;
        debug!("Enabling write-ahead log");
        conn.pragma_update(None, "journal_mode", "WAL")?;

        let tx = conn.transaction()?;
        Self::initialize(&tx)?;
        tx.commit()?;

        Ok(RecordDatabase { conn })
    }

    /// Check the current schema version.
    pub fn schema_version(tx: &Transaction) -> Result<i64, rusqlite::Error> {
        tx.pragma_query_value(None, "user_version", |row| row.get(0))
    }

    /// Initialize the relevant tables, or migrate from an older schema if necessary.
    fn initialize(tx: &Transaction) -> Result<(), DatabaseError> {
        debug!("Checking schema version");
        let db_schema_version = Self::schema_version(tx)?;

        if db_schema_version == schema_version() as i64 {
            Self::initialize_table(tx, "Records", sql::init_records())?;
            Self::initialize_table(tx, "CitationKeys", sql::init_citation_keys())?;
            Self::initialize_table(tx, "NullRecords", sql::init_null_records())?;
            Self::initialize_table(tx, "Changelog", sql::init_changelog())?;
            Ok(())
        } else {
            #[allow(clippy::match_single_binding)]
            match db_schema_version {
                // call a migration function here, if there are more valid versions
                _ => Err(DatabaseError::InvalidSchemaVersion(db_schema_version)),
            }
        }
    }

    /// Initialize a table inside a transaction.
    fn initialize_table(
        tx: &Transaction,
        table_name: &str,
        schema: &str,
    ) -> Result<(), DatabaseError> {
        debug!("Initializing new or validating existing table '{table_name}'");
        match Self::check_table_schema(tx, table_name, schema) {
            Ok(()) => Ok(()),
            Err(DatabaseError::TableMissing(_)) => {
                tx.execute(schema, ())?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Validate the schema of an existing table, or return an appropriate error.
    fn check_table_schema(
        tx: &Transaction,
        table_name: &str,
        expected_schema: &str,
    ) -> Result<(), DatabaseError> {
        let mut table_selector = tx.prepare(sql::get_table_schema())?;
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

    /// Get the [`RecordIdState`] associated with a [`RecordId`].
    #[inline]
    pub fn state_from_record_id(
        &mut self,
        record_id: RecordId,
    ) -> Result<RecordIdState, rusqlite::Error> {
        RecordIdState::determine(self.conn.transaction()?, record_id)
    }

    /// Get the [`RemoteIdState`] associated with a [`RemoteId`].
    #[inline]
    pub fn state_from_remote_id(
        &mut self,
        remote_id: &RemoteId,
    ) -> Result<RemoteIdState, rusqlite::Error> {
        RemoteIdState::determine(self.conn.transaction()?, remote_id)
    }

    /// Optimize the database.
    ///
    /// This should be called when the database connection is closed, or periodically during
    /// long-running operation.
    ///
    /// See the [SQLite docs](https://www.sqlite.org/pragma.html#pragma_optimize)
    /// for more detail.
    pub fn optimize(&mut self) -> Result<(), rusqlite::Error> {
        debug!("Optimizing database");
        self.conn.execute(sql::optimize(), ())?;
        Ok(())
    }

    /// Validate the internal consistency of the database.
    ///
    /// This does not modify the database.
    pub fn validate(&mut self) -> Result<(), ValidationError> {
        let validator = DatabaseValidator {
            tx: self.conn.transaction()?,
        };
        validator.record_indexing()?;
        validator.consistency()?;
        validator.record_data()?;
        validator.commit()?;
        Ok(())
    }

    /// Send the contents of the `Records` table to a [`Nucleo`](`nucleo_picker::nucleo::Nucleo`)
    /// instance via its [`Injector`].
    ///
    /// The `render_row` argument is a closure which accepts a row from the Records table, which
    /// consists of the [`RawRecordData`] along with a reference to the corresponding `CanonicalId`
    /// and the time it was last modified. The return type is used as the search string for the
    /// corresponding [`Nucleo`](`nucleo_picker::nucleo::Nucleo`) instance.
    ///
    /// Note, for instance, that [`String`] implements [`Into<Utf32String>`].
    pub fn inject_all_records<T, R>(
        &mut self,
        injector: Injector<RemoteId>,
        mut render_row: R,
    ) -> Result<(), rusqlite::Error>
    where
        T: Into<Utf32String>,
        R: FnMut(RawRecordData, &RemoteId, DateTime<Local>) -> T,
    {
        debug!("Sending all database records to an injector.");
        let mut retriever = self.conn.prepare(sql::get_all_records())?;

        for res in retriever.query_map([], |row| RowData::try_from(row))? {
            let RowData {
                data,
                canonical,
                modified,
            } = res?;
            let search_display: Utf32String = render_row(data, &canonical, modified).into();

            let _ = injector.push(canonical, move |_, cols| {
                cols[0] = search_display;
            });
        }

        Ok(())
    }

    /// Iterate over all names in the CitationKeys table and apply the infallible function
    /// `f` to each key.
    ///
    /// If `canonical` is true, only iterate over canonical keys.
    pub fn map_citation_keys<F: FnMut(&str)>(
        &mut self,
        canonical: bool,
        mut f: F,
    ) -> Result<(), rusqlite::Error> {
        let mut selector = if canonical {
            self.conn.prepare(sql::get_all_canonical_citation_keys())
        } else {
            self.conn.prepare(sql::get_all_citation_keys())
        }?;

        selector
            .query_map([], |row| {
                if let ValueRef::Text(bytes) = row.get_ref_unwrap(0) {
                    // SAFETY: the underlying data is always valid utf-8
                    f(unsafe { std::str::from_utf8_unchecked(bytes) });
                }
                Ok(())
            })?
            .for_each(drop);

        Ok(())
    }

    /// Rename an alias, returning the status of the renaming.
    pub fn rename_alias(
        &mut self,
        old: &Alias,
        new: &Alias,
    ) -> Result<RenameAliasResult, rusqlite::Error> {
        let mut updater = self.conn.prepare(sql::rename_citation_key())?;
        match flatten_constraint_violation(updater.execute((new.name(), old.name())))? {
            Constraint::Satisfied(_) => Ok(RenameAliasResult::Renamed),
            Constraint::Violated => Ok(RenameAliasResult::TargetExists),
        }
    }

    /// Delete an alias, returning the status of the deletion.
    pub fn delete_alias(&mut self, alias: &Alias) -> Result<DeleteAliasResult, rusqlite::Error> {
        let mut deleter = self.conn.prepare(sql::delete_citation_key())?;
        if deleter.execute((alias.name(),))? == 0 {
            Ok(DeleteAliasResult::Missing)
        } else {
            Ok(DeleteAliasResult::Deleted)
        }
    }
}

impl Drop for RecordDatabase {
    fn drop(&mut self) {
        if let Err(err) = self.optimize() {
            eprintln!("Failed to optimize database on close: {err}");
        }
    }
}

/// Take the result of a SQLite operation and extract a constraint violation.
pub fn flatten_constraint_violation<T>(
    res: Result<T, rusqlite::Error>,
) -> Result<Constraint<T>, rusqlite::Error> {
    match res {
        Ok(t) => Ok(Constraint::Satisfied(t)),
        Err(err) => match err.sqlite_error_code() {
            Some(rusqlite::ErrorCode::ConstraintViolation) => Ok(Constraint::Violated),
            _ => Err(err),
        },
    }
}

/// The outcome of flattening a constraint violation error.
pub enum Constraint<T> {
    /// All constraints were satisfied during the database operation; result of the operation.
    Satisfied(T),
    /// A constraint was not satisfied.
    Violated,
}

/// The result of renaming an alias.
pub enum RenameAliasResult {
    /// The alias was successfully renamed.
    Renamed,
    /// The new alias name already exists.
    TargetExists,
}

/// The result of renaming an alias.
pub enum DeleteAliasResult {
    /// The alias was successfully renamed.
    Deleted,
    /// The alias did not exist.
    Missing,
}
