mod operations;

use chrono::Local;
use log::debug;
use rusqlite::Transaction;

use super::{get_row_id, sql, CitationKey, RowData, RowId};
use crate::{RawRecordData, RemoteId};
pub use operations::*;

/// A Wrapper type to represent a row in the `Records` table which either exists
/// or is missing.
#[derive(Debug)]
pub enum DatabaseEntry<'conn> {
    /// The row exists.
    Exists(RecordRow<'conn>),
    /// The row is missing.
    Missing(MissingRecordRow<'conn>),
}

impl<'conn> DatabaseEntry<'conn> {
    /// Initialize a new [`DatabaseEntry`] given a transaction and the [`CitationKey`]
    /// corresponding to the row which either exists or is missing from the `Records` table.
    #[inline]
    pub(super) fn from_tx<K: CitationKey>(
        tx: Transaction<'conn>,
        key: &K,
    ) -> Result<Self, rusqlite::Error> {
        match get_row_id(&tx, key)? {
            Some(row_id) => {
                debug!("Beginning new transaction for row '{row_id}'.");
                Ok(DatabaseEntry::Exists(RecordRow::new(tx, row_id)))
            }
            None => {
                debug!("Beginning new generic transaction.");
                Ok(DatabaseEntry::Missing(MissingRecordRow::new(tx)))
            }
        }
    }
}

/// A private trait which allows the caller to prepare (cached) sql statements.
trait DatabaseTransaction {
    fn prepare(&self, sql: &str) -> Result<rusqlite::Statement, rusqlite::Error>;

    fn prepare_cached(&self, sql: &str) -> Result<rusqlite::CachedStatement, rusqlite::Error>;
}

/// A representation of a row in the `Records` table which is missing.
#[derive(Debug)]
pub struct MissingRecordRow<'conn> {
    tx: Transaction<'conn>,
}

impl<'conn> MissingRecordRow<'conn> {
    fn new(tx: Transaction<'conn>) -> Self {
        Self { tx }
    }

    /// Apply an operation to the database.
    #[inline]
    pub fn apply<T, O: FnOnce(&Self) -> Result<T, rusqlite::Error>>(
        &self,
        operation: O,
    ) -> Result<T, rusqlite::Error> {
        operation(self)
    }

    /// Create the row, converting into a [`RecordRow`].
    pub fn insert(
        self,
        data: &RawRecordData,
        canonical: &RemoteId,
    ) -> Result<RecordRow<'conn>, rusqlite::Error> {
        debug!("Inserting data for canonical id '{canonical}'");
        self.tx.prepare_cached(sql::set_cached_data())?.execute((
            canonical.name(),
            data.to_byte_repr(),
            &Local::now(),
        ))?;
        let row_id = self.tx.last_insert_rowid();
        Ok(RecordRow::new(self.tx, row_id))
    }

    /// Create the row and also insert a link in the `CitationKeys` table, converting into a [`RecordRow`].
    pub fn insert_and_ref(
        self,
        data: &RawRecordData,
        canonical: &RemoteId,
    ) -> Result<RecordRow<'conn>, rusqlite::Error> {
        let row = self.insert(data, canonical)?;
        row.apply(add_refs(std::iter::once(canonical)))?;
        Ok(row)
    }

    /// Commit the changes to the database.
    #[inline]
    pub fn commit(self) -> Result<(), rusqlite::Error> {
        debug!("Committing changes to database.");
        self.tx.commit()
    }

    /// Reset the row, clearing any internal data but preserving the transaction.
    pub fn reset<K: CitationKey>(self, key: &K) -> Result<DatabaseEntry<'conn>, rusqlite::Error> {
        DatabaseEntry::from_tx(self.tx, key)
    }
}

impl DatabaseTransaction for MissingRecordRow<'_> {
    #[inline]
    fn prepare(&self, sql: &str) -> Result<rusqlite::Statement, rusqlite::Error> {
        self.tx.prepare(sql)
    }

    #[inline]
    fn prepare_cached(&self, sql: &str) -> Result<rusqlite::CachedStatement, rusqlite::Error> {
        self.tx.prepare_cached(sql)
    }
}

/// A representation of a row in the `Records` table which exists.
#[derive(Debug)]
pub struct RecordRow<'conn> {
    tx: Transaction<'conn>,
    row_id: RowId,
}

impl<'conn> RecordRow<'conn> {
    /// Initialize a new [`RecordRow`].
    fn new(tx: Transaction<'conn>, row_id: RowId) -> Self {
        Self { tx, row_id }
    }

    /// Get the internal row id.
    #[inline]
    fn row_id(&self) -> RowId {
        self.row_id
    }

    /// Apply a database operation to the [`RecordRow`].
    #[inline]
    pub fn apply<T, O: FnOnce(&Self) -> Result<T, rusqlite::Error>>(
        &self,
        operation: O,
    ) -> Result<T, rusqlite::Error> {
        operation(self)
    }

    /// Delete the row, and convert to a [`MissingRecordRow`].
    pub fn delete(self) -> Result<MissingRecordRow<'conn>, rusqlite::Error> {
        debug!("Deleting data for row '{}'", self.row_id);
        self.apply(save_row_to_changelog)?;
        self.tx
            .prepare(sql::delete_cached_data())?
            .execute((self.row_id,))?;
        Ok(MissingRecordRow::new(self.tx))
    }

    /// Commit the changes to the database.
    #[inline]
    pub fn commit(self) -> Result<(), rusqlite::Error> {
        debug!("Committing changes to database.");
        self.tx.commit()
    }
}

impl DatabaseTransaction for RecordRow<'_> {
    #[inline]
    fn prepare(&self, sql: &str) -> Result<rusqlite::Statement, rusqlite::Error> {
        self.tx.prepare(sql)
    }

    #[inline]
    fn prepare_cached(&self, sql: &str) -> Result<rusqlite::CachedStatement, rusqlite::Error> {
        self.tx.prepare_cached(sql)
    }
}
