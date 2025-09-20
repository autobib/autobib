//! # Core database implementation
//! This module implements the abstraction over the underlying [SQLite](https://sqlite.org/)
//! database in which all bibliographic data is stored.
//!
//! The core struct is the [`RecordDatabase`].
//!
//! In order to represent internal database state, see the [`state`] module, along with its
//! documentation.
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
//! # record_data.check_and_insert("year".into(), "2023".into()).unwrap();
//! # record_data
//! #     .check_and_insert("title".into(), "The Title".into())
//! #     .unwrap();
//! # let byte_repr = RawRecordData::from(&record_data).into_byte_repr();
//! let expected = vec![
//!     0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 5, 9, 0, b't', b'i', b't', b'l', b'e',
//!     b'T', b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e', b'a', b'r',
//!     b'2', b'0', b'2', b'3',
//! ];
//! # assert_eq!(expected_byte_repr, byte_repr);
//! ```
mod functions;
mod migrate;
mod schema;
mod sql;
pub mod state;
mod validate;

use std::path::Path;

use chrono::{Local, TimeDelta};
use delegate::delegate;
use functions::{AppFunction, register_application_function};
use nucleo_picker::{Injector, Render};
use rusqlite::{Connection, DropBehavior, OpenFlags, OptionalExtension, types::ValueRef};

use self::state::{RecordIdState, RemoteIdState, RowData};
use self::validate::{DatabaseFault, DatabaseValidator};
use crate::{
    Alias, RecordId, RemoteId,
    config::AliasTransform,
    error::DatabaseError,
    logger::{debug, error, info, warn},
};

/// The current database version expected by the application.
pub const fn user_version() -> i32 {
    1
}

/// The unique application id used to determine if the opened database matches one used by this
/// application.
pub const fn application_id() -> i32 {
    // first 32 bits of sha256 hash of "Autobib"
    0x16611f2f
}

/// An alias for the internal row ID used by SQLite for the `Records` and the `NullRecords` table. This is
/// the `key` column in the table schema defined in [`schema::records`], and the
/// implicit `rowid` column in the table schema defined in [`schema::null_records`]
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
    impl<T> Sealed for crate::MappedKey<T> {}
}

/// Internal representation of the underlying SQL database.
///
/// The table structure is as follows.
///
/// 1. `Records`. This is the primary table used to store records. The integer primary key
///    `key` is used as the internal unambiguous reference for each record and is used for
///    de-duplication. The table schema is documented in [`schema::records`].
/// 2. `CitationKeys`. This is the table used to store any citation key which is inserted into
///    a table. Since multiple citation keys may refer to the same underlying record, this is
///    simply a lookup table for the corresponding record, and the corresponding rows are
///    automatically deleted when the record is deleted. The table schema is documented in
///    [`schema::citation_keys`].
/// 3. `NullRecords`. This is a cache table used to keep track of records which are known to
///    not exist. The table schema is documented in [`schema::null_records`].
///
/// For a [`RemoteId`], there are two variants depending on the value returned by [`get_remote_response`](crate::provider::get_remote_response):
///
/// - Canonical: if the return type is
///   [`RemoteResponse::Data`](crate::provider::RemoteResponse::Data).
/// - Reference: if the return type is
///   [`RemoteResponse::Reference`](crate::provider::RemoteResponse::Reference).
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
    /// If `read_only` is false, does the following initialization:
    /// - Checks the `application_id` to match the program ID.
    /// - Checks the `user_version`, migrating older versions and failing if the database version
    ///   is newer than the one expected by this binary.
    /// - If the database is empty (more precisely, if `sqlite_master` contains no entries)
    ///   initialize the expected tables as detailed in the documentation for [`RecordDatabase`].
    ///
    /// Any tables other than the expected tables are ignored.
    pub fn open<P: AsRef<Path>>(db_path: P, read_only: bool) -> Result<Self, DatabaseError> {
        debug!(
            "Initializing new connection to '{}'",
            db_path.as_ref().display()
        );
        let flags = if read_only {
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
        };
        #[cfg(not(feature = "write_response_cache"))]
        let mut conn = Connection::open_with_flags(db_path, flags)?;

        #[cfg(feature = "write_response_cache")]
        let mut conn = Connection::open_in_memory_with_flags(flags)?;

        Self::initialize(&mut conn, read_only)?;

        Ok(Self { conn })
    }

    /// Enable an application function for use in subsequent SQL queries.
    pub fn register_application_function(&self, fun: AppFunction) -> Result<(), DatabaseError> {
        debug!("Enabling application function: {}", fun.name());
        register_application_function(&self.conn, fun)?;
        Ok(())
    }

    /// Read the user version from the database connection.
    fn read_user_version(conn: &mut Connection) -> Result<i32, rusqlite::Error> {
        conn.pragma_query_value(None, "user_version", |row| row.get(0))
    }

    /// Read the user version of the current database.
    pub fn user_version(&mut self) -> Result<i32, rusqlite::Error> {
        Self::read_user_version(&mut self.conn)
    }

    /// Read the application id from the database connection.
    fn read_application_id(conn: &mut Connection) -> Result<i32, rusqlite::Error> {
        conn.pragma_query_value(None, "application_id", |row| row.get(0))
    }

    /// Check if the database at the provided connection is empty by checking that it contains no
    /// on-disk tables.
    fn is_empty_database(conn: &mut Connection) -> Result<bool, DatabaseError> {
        debug!("Checking if database is empty");
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master")?;
        let mut rows = stmt.query([])?;
        Ok(rows.next()?.is_none())
    }

    /// Initialize the relevant tables, or migrate from an older schema if necessary.
    fn initialize(conn: &mut Connection, read_only: bool) -> Result<(), DatabaseError> {
        let db_user_version = Self::read_user_version(conn)?;
        let db_application_id = Self::read_application_id(conn)?;
        debug!(
            "Database has user version '{db_user_version}' and application id '{db_application_id}'"
        );

        // fast path: `user_version` and `application_id` are both set and equal to the
        // versions for this binary
        if db_user_version == user_version() && db_application_id == application_id() {
            return Ok(());
        }

        // next most likely path: initializing a new database
        if Self::is_empty_database(conn)? && db_user_version == 0 && db_application_id == 0 {
            if read_only {
                return Err(DatabaseError::EmptyReadOnly);
            } else {
                info!("Creating new database");
                let tx = conn.transaction()?;

                debug!("Setting `application_id` and `user_version`");
                tx.pragma_update(None, "application_id", application_id())?;
                tx.pragma_update(None, "user_version", user_version())?;

                debug!("Initializing database tables");
                tx.execute(schema::records(), ())?;
                tx.execute(schema::citation_keys(), ())?;
                tx.execute(schema::null_records(), ())?;
                tx.execute(schema::changelog(), ())?;
                tx.commit()?;

                debug!("Enabling write-ahead log");
                conn.pragma_update(None, "journal_mode", "WAL")?;

                return Ok(());
            }
        };

        // check if the application id belongs to some other application
        if db_user_version < 0
            || (db_user_version == 0 && db_application_id != 0)
            || (db_user_version > 0 && db_application_id != application_id())
        {
            return Err(DatabaseError::InvalidDatabase);
        }

        // if read-only, we open the database and hope for the best; the worst case scenario
        // is that SQL commands will result in an error or garbage data
        if read_only {
            warn!(
                "Opening database (read-only) with version {}; application has version {}. This may result some commands to fail unexpectedly.",
                db_user_version,
                user_version()
            );
            return Ok(());
        }

        // check if the database version is too new
        if db_user_version > user_version() {
            return Err(DatabaseError::DatabaseVersionNewerThanBinary(
                db_user_version,
                user_version(),
            ));
        }

        // by now, we have checked that:
        // - the database is non-empty
        // - the `application_id` is equal to the one for this program
        // - the `user_version` is strictly less than the user version of this binary
        for v in db_user_version..user_version() {
            // apply the migration code for each previous version
            //
            // note that the migration code for `v0` automatically checks the database for validity
            // of tables
            migrate::migrate(conn, v)?;
        }
        Ok(())
    }

    /// Execute [sqlite VACUUM](https://www.sqlite.org/lang_vacuum.html).
    pub fn vacuum(&mut self) -> Result<(), rusqlite::Error> {
        self.conn.execute("VACUUM", ()).map(|_| ())
    }

    /// Get the [`RecordIdState`] associated with a [`RecordId`].
    #[inline]
    pub fn extended_state_from_record_id<A: AliasTransform>(
        &mut self,
        record_id: RecordId,
        alias_transform: &A,
    ) -> Result<RecordIdState<'_>, rusqlite::Error> {
        RecordIdState::determine(self.conn.transaction()?.into(), record_id, alias_transform)
    }

    /// Get the [`RecordIdState`] associated with a [`RecordId`].
    #[inline]
    pub fn state_from_record_id<A: AliasTransform>(
        &mut self,
        record_id: RecordId,
        alias_transform: &A,
    ) -> Result<RecordIdState<'_>, rusqlite::Error> {
        RecordIdState::determine(self.conn.transaction()?.into(), record_id, alias_transform)
    }

    /// Get the [`RemoteIdState`] associated with a [`RemoteId`].
    #[inline]
    pub fn state_from_remote_id(
        &mut self,
        remote_id: &RemoteId,
    ) -> Result<RemoteIdState<'_>, rusqlite::Error> {
        RemoteIdState::determine(self.conn.transaction()?.into(), remote_id)
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
    /// If `fix` is true, then potentially destructive database changes will take place.
    pub fn recover(&mut self, fix: bool) -> Result<Vec<DatabaseFault>, rusqlite::Error> {
        let validator = DatabaseValidator {
            tx: self.conn.transaction()?.into(),
        };
        let mut faults = Vec::new();

        validator.table_schema(&mut faults)?;
        validator.record_indexing(&mut faults)?;
        validator.invalid_citation_keys(&mut faults)?;
        validator.integrity(&mut faults)?;
        validator.binary_data(&mut faults)?;

        let tx = validator.into_tx();

        if fix {
            faults.retain(|fault| match Self::fix_fault_tx(&tx, fault) {
                Ok(b) => !b,
                Err(err) => {
                    error!("While fixing the error {fault}, another error occurred:\n  {err}");
                    false
                }
            });
        }

        tx.commit()?;

        Ok(faults)
    }

    /// Attempt to fix a database fault inside a transaction.
    ///
    /// If the fault is fixed, return `true`, and return `false` otherwise.
    fn fix_fault_tx(tx: &Transaction, fault: &DatabaseFault) -> Result<bool, rusqlite::Error> {
        match fault {
            DatabaseFault::RowHasInvalidCanonicalId(_, _) => Ok(false),
            DatabaseFault::DanglingRecord(key, canonical) => {
                warn!(
                    "Repairing dangling record by inserting or overwriting existing citation key with name {canonical}"
                );
                tx.prepare(sql::set_citation_key_overwrite())?
                    .execute((canonical, key))?;
                Ok(true)
            }
            DatabaseFault::NullCitationKeys(_) => {
                let mut invalid_keys: Vec<String> = Vec::new();
                {
                    let mut stmt = tx.prepare(
                                        "SELECT name FROM CitationKeys WHERE record_key NOT IN (SELECT key FROM Records)",
                                    )?;
                    let mut rows = stmt.query(())?;
                    while let Some(row) = rows.next()? {
                        invalid_keys.push(row.get("name")?);
                    }
                }

                warn!("Deleting citation keys which do not reference records:");
                for name in invalid_keys {
                    eprintln!("  {name}");
                }
                tx.prepare(
                    "DELETE FROM CitationKeys WHERE record_key NOT IN (SELECT key FROM Records)",
                )?
                .execute(())?;
                Ok(true)
            }
            DatabaseFault::IntegrityError(_) => Ok(false),
            DatabaseFault::InvalidRecordData(_, _, _) => Ok(false),
            DatabaseFault::RowHasNonNormalizedCanonicalId(_, _, _) => Ok(false),
            DatabaseFault::InvalidCitationKey(_) => Ok(false),
            DatabaseFault::NonNormalizedCitationKey(_, _) => Ok(false),
            DatabaseFault::MissingTable(_) => Ok(false),
            DatabaseFault::InvalidTableSchema(_, _) => Ok(false),
        }
    }

    /// Send the contents of the `Records` table to a [`Picker`](`nucleo_picker::Picker`)
    /// via its [`Injector`].
    ///
    /// This is a convenience wrapper around [`Self::inject_records`] which simply sends all row data
    /// to the picker without filtering or mapping.
    pub fn inject_all_records<R: Render<RowData>>(
        &mut self,
        injector: Injector<RowData, R>,
    ) -> Result<(), rusqlite::Error> {
        self.inject_records(injector, Some)
    }

    /// Send the contents of the `Records` table to a [`Picker`](`nucleo_picker::Picker`)
    /// via its [`Injector`].
    ///
    /// The provided `filter_map` closure plays a similar role to [`Iterator::filter_map`]
    /// by transforming a [`RowData`] into the picker item type, with the option to exclude
    /// the item from being sent to the matcher entirely by returning [`None`].
    pub fn inject_records<T, F: FnMut(RowData) -> Option<T>, R: Render<T>>(
        &mut self,
        injector: Injector<T, R>,
        mut filter_map: F,
    ) -> Result<(), rusqlite::Error> {
        debug!("Sending all database records to an injector.");
        let mut retriever = self.conn.prepare(sql::get_all_records())?;

        for res in retriever.query_map([], |row| RowData::try_from(row))? {
            if let Some(data) = filter_map(res?) {
                injector.push(data);
            }
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
                    f(std::str::from_utf8(bytes).unwrap());
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

    /// Delete all rows from `NullRecords`.
    pub fn evict_cache(&mut self) -> Result<(), rusqlite::Error> {
        let num_deleted = self.conn.prepare(sql::clear_null_records())?.execute(())?;
        info!("Removed {num_deleted} cached null records.");
        Ok(())
    }

    /// Delete all rows from `NullRecords` which are at least a given age (in seconds)
    pub fn evict_cache_max_age(&mut self, seconds: u32) -> Result<(), rusqlite::Error> {
        let threshold = Local::now() - TimeDelta::seconds(seconds.into());
        let num_deleted = self
            .conn
            .prepare(sql::clear_null_records_before())?
            .execute((threshold,))?;
        info!("Removed {num_deleted} cached null records.");
        Ok(())
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
#[must_use]
pub enum RenameAliasResult {
    /// The alias was successfully renamed.
    Renamed,
    /// The new alias name already exists.
    TargetExists,
}

/// The result of renaming an alias.
#[must_use]
pub enum DeleteAliasResult {
    /// The alias was successfully renamed.
    Deleted,
    /// The alias did not exist.
    Missing,
}

/// Custom wrapper around a [`rusqlite::Transaction`] to provide additional logging.
#[derive(Debug)]
pub struct Transaction<'conn> {
    tx: rusqlite::Transaction<'conn>,
}

impl<'conn> From<rusqlite::Transaction<'conn>> for Transaction<'conn> {
    fn from(tx: rusqlite::Transaction<'conn>) -> Self {
        Self { tx }
    }
}

impl Transaction<'_> {
    /// Commit the transaction.
    ///
    /// This method sets the transaction's drop behaviour to [`rusqlite::DropBehavior::Commit`] and then drops it.
    pub fn commit(mut self) -> rusqlite::Result<()> {
        self.tx.set_drop_behavior(DropBehavior::Commit);
        drop(self);
        Ok(())
    }

    delegate! {
        to self.tx {
            pub fn execute<P>(&self, sql: &str, params: P) ->rusqlite::Result<usize>
            where
                P: rusqlite::Params;
            pub fn last_insert_rowid(&self) -> i64;
            pub fn pragma_query<F>(&self, schema_name: Option<&str>, pragma_name: &str, f: F) -> rusqlite::Result<()>
            where
                F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<()>;
            pub fn pragma_query_value<T, F>(&self, schema_name: Option<&str>, pragma_name: &str, f: F) -> rusqlite::Result<T>
            where
                F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>;
            pub fn prepare(&self, sql: &str) -> rusqlite::Result<rusqlite::Statement<'_>>;
            pub fn prepare_cached(&self, sql: &str) -> rusqlite::Result<rusqlite::CachedStatement<'_>>;
        }
    }
}

impl Drop for Transaction<'_> {
    #[inline]
    fn drop(&mut self) {
        match self.tx.drop_behavior() {
            DropBehavior::Rollback => debug!("Rolling back transaction"),
            DropBehavior::Commit => debug!("Committing transaction"),
            DropBehavior::Ignore => debug!("Ignoring transaction"),
            DropBehavior::Panic => debug!("Dropping transaction and panicking"),
            _ => debug!("Dropping transaction with unknown drop behaviour"),
        }
    }
}
