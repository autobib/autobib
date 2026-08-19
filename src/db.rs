//! # Core database implementation
//!
//! This module implements the abstraction over the underlying [SQLite](https://sqlite.org/)
//! database in which all bibliographic data is stored.
//!
//! The core struct is the [`RecordDatabase`]. There are two primary abstractions around a
//! SQL transaction:
//!
//! - The [`state`] module implements abstraction for a *single identifier*, such as a
//!   row in the 'Records' table, a row in the `NullRecords' table, or if the identifier is not
//!   present in the database at all
//! - The [`Snapshot`] struct represents a global representation of database state.

mod functions;
mod migrate;
mod schema;
pub mod select;
mod snapshot;
pub mod state;
pub mod tree;
mod validate;

use std::path::Path;

use autobib_entry::{Archive, data::MutableEntryData, v1::ArchivedEntryData};
use delegate::delegate;
use functions::{AppFunction, register_application_function};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use self::{
    state::{DatabaseIdResponse, DatabaseResponse, Record},
    validate::{DatabaseFault, DatabaseValidator},
};
use crate::{
    Identifier, Key,
    config::AliasTransform,
    error::DatabaseError,
    logger::{debug, info, warn},
};
pub use snapshot::{
    Constraint, DeleteAliasResult, RenameAliasResult, Snapshot, flatten_constraint_violation,
};

/// The current database version expected by the application.
pub const fn user_version() -> i32 {
    5
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

/// Determine the [`RowId`] in the `Records` table corresponding to an [`Identifier`].
fn get_row_id<K: AsKey>(tx: &Tx, key: &K) -> Result<Option<RowId>, rusqlite::Error> {
    tx.prepare_cached("SELECT record_rev FROM Keys WHERE name = ?1")?
        .query_row([key.as_key()], |row| row.get("record_rev"))
        .optional()
}

/// Determine the [`RowId`] in the `NullRecords` table corresponding to an [`Identifier`].
pub fn get_null_row_id(tx: &Tx, id: &Identifier) -> Result<Option<RowId>, rusqlite::Error> {
    tx.prepare_cached("SELECT rowid FROM NullRecords WHERE canonical = ?1")?
        .query_row([id.as_key()], |row| row.get("rowid"))
        .optional()
}

/// This trait represents types which can be stored as a row in the Keys table
pub trait AsKey: private::Sealed {
    /// The string to use as the key for a row.
    fn as_key(&self) -> &str;
}

mod private {
    /// Prevent implemntation of [`AsKey`](super::AsKey) by foreign types.
    pub trait Sealed {}

    impl Sealed for crate::Alias {}
    impl Sealed for crate::Key {}
    impl<S: AsRef<str>> Sealed for crate::Identifier<S> {}
    impl<T> Sealed for crate::MappedKey<T> {}
}

/// Internal representation of the underlying SQL database.
///
/// The table structure is as follows.
///
/// 1. `Records`. This is the primary table used to store records. The integer primary key
///    `key` is used as the internal unambiguous reference for each record and is used for
///    de-duplication. The table schema is documented in [`schema::records`].
/// 2. `Keys`. This is the table used to store lookup keys for records. The corresponding
///    rows are automatically deleted when the record is deleted. The table schema is
///    documented in [`schema::keys`].
/// 3. `NullRecords`. This is a cache table used to keep track of records which are known to
///    not exist. The table schema is documented in [`schema::null_records`].
///
/// For an [`Identifier`], there are two variants depending on the value returned by [`get_remote_response`](crate::provider::get_remote_response):
///
/// - Canonical: if the return type is
///   [`RemoteResponse::Data`](crate::provider::RemoteResponse::Data).
/// - Reference: if the return type is
///   [`RemoteResponse::Reference`](crate::provider::RemoteResponse::Reference).
///
/// This distinction is not currently enforced by types, but it may be in the future.
///
/// The two identifier types, [`Alias`](crate::record::Alias) and [`Identifier`], with the "Canonical" and "Reference"
/// for [`Identifier`], are stored according to the following table.
///
/// |            | Stored in Records | Stored in NullRecords | Stored in Keys |
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
        #[cfg(not(feature = "in_memory_database"))]
        let mut conn = Connection::open_with_flags(db_path, flags)?;

        #[cfg(feature = "in_memory_database")]
        let mut conn = Connection::open_in_memory_with_flags(flags)?;

        #[cfg(not(feature = "bundled-sqlite"))]
        {
            if !read_only {
                // 3.35, since this is when the RETURNING class is introduced
                if rusqlite::version_number() < 3_035_000 {
                    return Err(DatabaseError::UnsupportedSQLiteVersion(rusqlite::version()));
                }

                // we only need this when using system sqlite since the bundled version is compiled
                // so that foreign keys are always enabled at startup
                conn.pragma_update(None, "foreign_keys", "ON")?;
            }
        }

        Self::initialize(&mut conn, read_only)?;

        Ok(Self { conn })
    }

    /// Obtain a [`Snapshot`], which provides options to modify the database within a transaction.
    /// Note that the database can be accessed immutably either through a snapshot or through this type
    /// directly; see [`Select`](select::Select).
    pub fn snapshot(&mut self) -> rusqlite::Result<Snapshot<'_>> {
        Ok(Snapshot {
            tx: self.conn.transaction()?.into(),
        })
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
                tx.execute(schema::keys(), ())?;
                tx.execute(schema::null_records(), ())?;

                debug!("Initializing indices");
                tx.execute_batch(schema::create_indices())?;

                debug!("Initializing views");
                tx.execute_batch(schema::create_views())?;

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
                "Opening database (read-only) with version {}; application has version {}. Some commands may fail unexpectedly.",
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

    /// Execute [sqlite VACUUM INTO](https://www.sqlite.org/lang_vacuum.html).
    pub fn vacuum_into<P: AsRef<Path>>(&mut self, into: P) -> Result<(), rusqlite::Error> {
        let Some(into_str) = into.as_ref().to_str() else {
            return Err(rusqlite::Error::InvalidPath(into.as_ref().to_owned()));
        };

        self.conn.execute("VACUUM INTO ?1", [into_str]).map(|_| ())
    }

    pub fn transaction(&mut self) -> rusqlite::Result<Tx<'_>> {
        self.conn.transaction().map(Into::into)
    }

    /// Get the [`DatabaseResponse`] associated with a [`Key`].
    #[inline]
    pub fn state_from_key<A: AliasTransform>(
        &mut self,
        key: Key,
        alias_transform: &A,
    ) -> Result<DatabaseResponse<'_>, rusqlite::Error> {
        DatabaseResponse::determine(self.transaction()?, key, alias_transform)
    }

    /// Get the [`DatabaseIdResponse`] associated with an [`Identifier`].
    #[inline]
    pub fn state_from_id(
        &mut self,
        id: &Identifier,
    ) -> Result<DatabaseIdResponse<'_>, rusqlite::Error> {
        DatabaseIdResponse::determine(self.transaction()?, id)
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
        self.conn.execute("PRAGMA optimize", ())?;
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
        validator.invalid_identifiers(&mut faults)?;
        validator.integrity(&mut faults)?;
        validator.binary_data(&mut faults)?;
        validator.unique_tree_per_key(&mut faults)?;
        validator.monotonic_timestamps(&mut faults)?;
        validator.void_correct_formatting(&mut faults)?;
        validator.check_active_row_counts(&mut faults)?;

        let tx = validator.into_tx();

        if fix {
            let mut unresolved = Vec::with_capacity(faults.len());
            for fault in faults {
                if !Self::fix_fault_tx(&tx, &fault)? {
                    unresolved.push(fault);
                }
            }
            faults = unresolved;
        }

        tx.commit()?;

        Ok(faults)
    }

    /// Attempt to fix a database fault inside a transaction.
    ///
    /// If the fault is fixed, return `true`, and return `false` otherwise.
    fn fix_fault_tx(tx: &Tx, fault: &DatabaseFault) -> Result<bool, rusqlite::Error> {
        // Notes for improving:
        // - Many changes require interactivity because the user should see the
        //   change and confirm it.
        // - Keys identified by `InvalidKey` can still be accessed, so they shouldn't
        //   be deleted / renamed without confirmation.
        // - Invalid revision tree structure (multiple disjoint trees; non-root voids), should
        //   be fixed as follows:
        //   - remove internal voids, connecting the child to the parent (if any)
        //   - check if there are disjoint trees, and merge them all to share the same
        //     void root, creating it if required
        //   - set the timestamp of the void root to UTC_MIN
        match fault {
            DatabaseFault::NonNormalizedId(current, normalized) => {
                let target_rev = tx
                    .prepare("SELECT record_rev FROM Keys WHERE name = ?1")?
                    .query_row([normalized], |row| row.get::<_, i64>(0))
                    .optional()?;
                let current_rev = tx
                    .prepare("SELECT record_rev FROM Keys WHERE name = ?1")?
                    .query_row([current], |row| row.get::<_, i64>(0))?;

                match target_rev {
                    None => {
                        warn!("Normalizing key '{current}' to '{normalized}'");
                        tx.prepare("UPDATE Keys SET name = ?1 WHERE name = ?2")?
                            .execute((normalized, current))?;
                        Ok(true)
                    }
                    Some(target_rev) if target_rev == current_rev => {
                        warn!(
                            "Deleting non-normalized key '{current}'; normalized key '{normalized}' already exists and references the same record"
                        );
                        tx.prepare("DELETE FROM Keys WHERE name = ?1")?
                            .execute([current])?;
                        Ok(true)
                    }
                    Some(_) => {
                        warn!(
                            "Cannot normalize identifier '{current}' to '{normalized}': the normalized identifier already exists"
                        );
                        Ok(false)
                    }
                }
            }
            DatabaseFault::InvalidRecordData(rev, _, _)
            | DatabaseFault::InvalidRecordDataFormat(rev, _, _) => {
                // 'from_archive_universal' reads from v0 or v1, and also fixes sorting errors.
                // these are the most likely data problems. other errors cannot be fixed
                let data = match tx
                    .prepare("SELECT data FROM Records WHERE rev = ?1 AND variant = 0")?
                    .query_row([rev], |row| {
                        let bytes = row.get_ref(0)?.as_blob()?;
                        MutableEntryData::from_archive_universal(bytes).map_err(|err| {
                            rusqlite::Error::FromSqlConversionFailure(
                                0,
                                rusqlite::types::Type::Blob,
                                Box::new(err),
                            )
                        })
                    }) {
                    Ok(data) => data,
                    Err(err) => {
                        warn!("Binary data format could not be repaired: {err}");
                        return Ok(false);
                    }
                };
                let repaired = ArchivedEntryData::from_entry_data(&data);

                tx.prepare("UPDATE Records SET data = ?1 WHERE rev = ?2 AND variant = 0")?
                    .execute((repaired.as_bytes(), rev))?;
                Ok(true)
            }
            DatabaseFault::NullKeys(_) => {
                let mut invalid_keys: Vec<String> = Vec::new();
                {
                    let mut stmt = tx.prepare(
                        "SELECT name FROM Keys WHERE record_rev NOT IN (SELECT rev FROM Records)",
                    )?;
                    let mut rows = stmt.query(())?;
                    while let Some(row) = rows.next()? {
                        invalid_keys.push(row.get("name")?);
                    }
                }

                warn!("Deleting identifiers which do not reference records:");
                for name in invalid_keys {
                    eprintln!("  {name}");
                }
                tx.prepare("DELETE FROM Keys WHERE record_rev NOT IN (SELECT rev FROM Records)")?
                    .execute(())?;
                Ok(true)
            }
            DatabaseFault::MissingTable(name) if name == "NullRecords" => {
                tx.execute(schema::null_records(), ())?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

impl Drop for RecordDatabase {
    fn drop(&mut self) {
        let _ = self.optimize();
    }
}

/// A wrapper around a [`rusqlite::Transaction`] which provides additional logging and exposes
/// fewer public methods.
#[derive(Debug)]
pub struct Tx<'conn> {
    tx: rusqlite::Transaction<'conn>,
    _drop_log: TxDropLog,
}

#[derive(Debug)]
struct TxDropLog;

/// A drop guard which writes a debug message on implicit rollback.
impl Drop for TxDropLog {
    fn drop(&mut self) {
        debug!("Implicitly rolling back transaction");
    }
}

impl core::ops::Deref for Tx<'_> {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        self.tx.deref()
    }
}

impl<'conn> From<rusqlite::Transaction<'conn>> for Tx<'conn> {
    fn from(tx: rusqlite::Transaction<'conn>) -> Self {
        Self {
            tx,
            _drop_log: TxDropLog,
        }
    }
}

impl Tx<'_> {
    pub fn inner_connection(&self) -> &Connection {
        &self.tx
    }

    /// Commit the transaction.
    pub fn commit(self) -> rusqlite::Result<()> {
        let Self { tx, _drop_log } = self;
        // suppress drop guard rollback message.
        std::mem::forget(_drop_log);
        debug!("Committing transaction");
        tx.commit()
    }

    /// Roll back the transaction.
    pub fn rollback(self) -> rusqlite::Result<()> {
        let Self { tx, _drop_log } = self;
        // suppress drop guard rollback message.
        std::mem::forget(_drop_log);
        debug!("Rolling back transaction");
        tx.rollback()
    }

    // only expose internal methods privately
    delegate! {
        to self.tx {
            fn pragma_query<F>(&self, schema_name: Option<&str>, pragma_name: &str, f: F) -> rusqlite::Result<()>
            where
                F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<()>;

            fn prepare(&self, sql: &str) -> rusqlite::Result<rusqlite::Statement<'_>>;

            fn prepare_cached(&self, sql: &str) -> rusqlite::Result<rusqlite::CachedStatement<'_>>;
        }
    }
}
