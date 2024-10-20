use chrono::{DateTime, Local};
use log::debug;
use rusqlite::Transaction;

use super::{
    flatten_constraint_violation, get_row_id, sql, CitationKey, Constraint, RowData, RowId,
};
use crate::{Alias, RawRecordData, RemoteId};

/// A Wrapper type to represent a row in the `Records` table which either exists
/// or is missing.
#[derive(Debug)]
pub enum DatabaseEntry<'conn> {
    /// The row exists.
    Exists(Row<'conn>),
    /// The row is missing.
    Missing(Missing<'conn>),
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
                Ok(DatabaseEntry::Exists(Row::new(tx, row_id)))
            }
            None => {
                debug!("Beginning new generic transaction.");
                Ok(DatabaseEntry::Missing(Missing::new(tx)))
            }
        }
    }
}

/// A representation of a row in the `Records` table which is missing.
#[derive(Debug)]
pub struct Missing<'conn> {
    tx: Transaction<'conn>,
}

impl<'conn> Missing<'conn> {
    fn new(tx: Transaction<'conn>) -> Self {
        Self { tx }
    }

    /// Apply an operation to the database.
    pub fn apply<T, O: FnOnce(&Transaction<'conn>) -> Result<T, rusqlite::Error>>(
        &self,
        operation: O,
    ) -> Result<T, rusqlite::Error> {
        operation(&self.tx)
    }

    /// Create the row, converting into a [`Row`].
    pub fn insert(
        self,
        data: &RawRecordData,
        canonical: &RemoteId,
    ) -> Result<Row<'conn>, rusqlite::Error> {
        debug!("Inserting data for canonical id '{canonical}'");
        self.tx.prepare_cached(sql::set_cached_data())?.execute((
            canonical.name(),
            data.to_byte_repr(),
            &Local::now(),
        ))?;
        let row_id = self.tx.last_insert_rowid();
        Ok(Row::new(self.tx, row_id))
    }

    /// Create the row and also insert a link in the `CitationKeys` table, converting into a [`Row`].
    pub fn insert_and_ref(
        self,
        data: &RawRecordData,
        canonical: &RemoteId,
    ) -> Result<Row<'conn>, rusqlite::Error> {
        let row = self.insert(data, canonical)?;
        row.apply(add_refs(std::iter::once(canonical)))?;
        Ok(row)
    }

    /// Commit the changes to the database.
    pub fn commit(self) -> Result<(), rusqlite::Error> {
        debug!("Committing changes to database.");
        self.tx.commit()
    }

    /// Reset the row, clearing any internal data but preserving the transaction.
    pub fn reset<K: CitationKey>(self, key: &K) -> Result<DatabaseEntry<'conn>, rusqlite::Error> {
        DatabaseEntry::from_tx(self.tx, key)
    }
}

/// A representation of a row in the `Records` table which is exists.
#[derive(Debug)]
pub struct Row<'conn> {
    tx: Transaction<'conn>,
    row_id: RowId,
}

impl<'conn> Row<'conn> {
    /// Initialize a new [`Row`].
    fn new(tx: Transaction<'conn>, row_id: RowId) -> Self {
        Self { tx, row_id }
    }

    /// Apply a database operation to the [`Row`].
    pub fn apply<T, O: FnOnce(&Transaction<'conn>, RowId) -> Result<T, rusqlite::Error>>(
        &self,
        operation: O,
    ) -> Result<T, rusqlite::Error> {
        operation(&self.tx, self.row_id)
    }

    /// Delete the row, and convert to a [`Missing`].
    pub fn delete(self) -> Result<Missing<'conn>, rusqlite::Error> {
        debug!("Deleting data for row '{}'", self.row_id);
        self.apply(save_row_to_changelog)?;
        self.tx
            .prepare(sql::delete_cached_data())?
            .execute((self.row_id,))?;
        Ok(Missing::new(self.tx))
    }

    /// Commit the changes to the database.
    pub fn commit(self) -> Result<(), rusqlite::Error> {
        debug!("Committing changes to database.");
        self.tx.commit()
    }
}

/// Delete the row (and therefore all referencing keys in `CitationKeys`) corresponding to the
/// [`Row`].
pub fn delete_row_data(tx: &Transaction, row_id: RowId) -> Result<(), rusqlite::Error> {
    debug!("Deleting row data for '{row_id}'.");
    save_row_to_changelog(tx, row_id)?;
    let mut updater = tx.prepare(sql::delete_cached_data())?;
    updater.execute((row_id,))?;
    Ok(())
}

/// Get every key in the `CitationKeys` table which references the [`Row`].
pub fn get_referencing_keys(
    tx: &Transaction,
    row_id: RowId,
) -> Result<Vec<String>, rusqlite::Error> {
    debug!("Getting referencing keys for '{row_id}'.");
    let mut selector = tx.prepare(sql::get_all_referencing_citation_keys())?;
    let rows = selector.query_map((row_id,), |row| row.get(0))?;
    let mut referencing = Vec::with_capacity(1);
    for name_res in rows {
        referencing.push(name_res?);
    }
    Ok(referencing)
}

/// Get the canonical [`RemoteId`] corresponding to a [`Row`].
pub fn get_canonical(tx: &Transaction, row_id: RowId) -> Result<RemoteId, rusqlite::Error> {
    debug!("Getting canonical identifier for '{row_id}'.");
    let RowData { canonical, .. } = get_row_data(tx, row_id)?;
    Ok(canonical)
}

/// Get the [`RowData`] corresponding to a [`Row`].
#[inline]
pub fn last_modified(tx: &Transaction, row_id: RowId) -> Result<DateTime<Local>, rusqlite::Error> {
    debug!("Getting data for row '{row_id}'.");
    let RowData { modified, .. } = get_row_data(tx, row_id)?;
    Ok(modified)
}

/// Get the [`RowData`] corresponding to a [`Row`].
#[inline]
pub fn get_row_data(tx: &Transaction, row_id: RowId) -> Result<RowData, rusqlite::Error> {
    debug!("Getting data for row '{row_id}'.");
    let mut record_selector = tx.prepare_cached(sql::get_cached_data())?;
    let mut record_rows = record_selector.query([row_id])?;
    record_rows
        .next()?
        .expect("RowId does not exist!")
        .try_into()
}

/// Copy the [`RowData`] of a row corresponding to a [`Row`] to the `Changelog` table.
pub fn save_row_to_changelog(tx: &Transaction, row_id: RowId) -> Result<(), rusqlite::Error> {
    debug!("Saving row '{row_id}' to Changelog table");
    tx.prepare_cached(sql::copy_to_changelog())?
        .execute((row_id,))?;
    Ok(())
}

/// Replace the [`RawRecordData`] corresponding to an existing [`Row`] with new data.
pub fn update_row_data(
    data: &RawRecordData,
) -> impl FnOnce(&Transaction, RowId) -> Result<(), rusqlite::Error> + '_ {
    move |tx, row_id| {
        debug!("Updating row data for row '{row_id}'");
        save_row_to_changelog(tx, row_id)?;
        let mut updater = tx.prepare(sql::update_cached_data())?;
        updater.execute((row_id, &Local::now(), data.to_byte_repr()))?;
        Ok(())
    }
}

/// Response type from the `NullRecords` table as returned by [`check_null`].
pub enum NullRecordsResponse {
    /// Null was found; last attempted.
    Found(DateTime<Local>),
    /// Null was not found.
    NotFound,
}

/// Check if a given [`RemoteId`] corresponds to a null record.
pub fn check_null(
    remote_id: &RemoteId,
) -> impl FnOnce(&Transaction) -> Result<NullRecordsResponse, rusqlite::Error> + '_ {
    move |tx| {
        debug!("Checking null entry for '{remote_id}'");
        let mut null_selector = tx.prepare_cached(sql::get_cached_null())?;
        let mut null_rows = null_selector.query([remote_id.name()])?;

        match null_rows.next()? {
            Some(row) => Ok(NullRecordsResponse::Found(row.get("attempted")?)),
            None => Ok(NullRecordsResponse::NotFound),
        }
    }
}

/// Insert [`RemoteId`]s into the `NullRecords` table.
pub fn set_null<'a, R: Iterator<Item = &'a RemoteId>>(
    remote_id_iter: R,
) -> impl FnOnce(&Transaction) -> Result<(), rusqlite::Error> {
    move |tx| {
        let mut setter = tx.prepare_cached(sql::set_cached_null())?;
        let cache_time = Local::now();
        for remote_id in remote_id_iter {
            debug!("Setting null entry for '{remote_id}'");
            setter.execute((remote_id.name(), cache_time))?;
        }

        Ok(())
    }
}

/// Add a new alias to the [`Row`].
///
/// The return value is `false` if the alias already exists, and otherwise `true`.
pub fn add_alias(
    alias: &Alias,
) -> impl FnOnce(&Transaction, RowId) -> Result<bool, rusqlite::Error> + '_ {
    add_refs_impl(std::iter::once(alias), CitationKeyInsertMode::FailIfExists)
}

/// Insert [`CitationKey`] references to the row corresponding to a [`Row`].
///
/// The return value is `false` if the insertion failed and `CitationKeyInsertMode` is
/// `FailIfExists`, and otherwise `true`.
pub fn add_refs<'a, R: Iterator<Item = &'a RemoteId>>(
    refs: R,
) -> impl FnOnce(&Transaction, RowId) -> Result<bool, rusqlite::Error> {
    add_refs_impl(refs, CitationKeyInsertMode::Overwrite)
}

/// The type of citation key insertion to perform.
pub enum CitationKeyInsertMode {
    /// Overwrite the existing citation key, if any.
    Overwrite,
    /// Fail if there is an existing citation key.
    FailIfExists,
    /// Ignore if there is an existing citation key.
    IgnoreIfExists,
}

/// Insert [`CitationKey`] references to the row corresponding to a [`Row`].
///
/// The return value is `false` if the insertion failed and `CitationKeyInsertMode` is
/// `FailIfExists`, and otherwise `true`.
#[inline]
fn add_refs_impl<'a, K: CitationKey + 'a, R: Iterator<Item = &'a K>>(
    refs: R,
    mode: CitationKeyInsertMode,
) -> impl FnOnce(&Transaction, RowId) -> Result<bool, rusqlite::Error> {
    move |tx, row_id| {
        debug!("Inserting references to row_id '{row_id}'");
        for remote_id in refs {
            let stmt = match mode {
                CitationKeyInsertMode::Overwrite => sql::set_citation_key_overwrite(),
                CitationKeyInsertMode::IgnoreIfExists => sql::set_citation_key_ignore(),
                CitationKeyInsertMode::FailIfExists => sql::set_citation_key_fail(),
            };
            let mut key_writer = tx.prepare(stmt)?;
            match flatten_constraint_violation(key_writer.execute((remote_id.name(), row_id)))? {
                Constraint::Satisfied(_) => {}
                Constraint::Violated => return Ok(false),
            }
        }
        Ok(true)
    }
}
