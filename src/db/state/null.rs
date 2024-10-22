use chrono::{DateTime, Local};
use log::debug;
use rusqlite::Transaction;

use super::{add_refs, transaction::tx_impl, DatabaseState, MissingRow, RecordRow};
use crate::{
    db::{sql, CitationKey, NullRowData, RowId},
    RawRecordData, RemoteId,
};

/// The database state when:
/// 1. There is no row in `Records`.
/// 2. There is a row in `NullRecords`.
///
/// The `row_id` is the corresponding rowid in the `NullRecords` table.
#[derive(Debug)]
pub struct NullRecordRow<'conn> {
    tx: Transaction<'conn>,
    row_id: RowId,
}

tx_impl!(NullRecordRow);

impl<'conn> NullRecordRow<'conn> {
    /// Initialize a new [`NullRecordRow`].
    pub(super) fn new(tx: Transaction<'conn>, row_id: RowId) -> Self {
        Self { tx, row_id }
    }

    /// Create the row, converting into a [`RecordRow`] and deleting the corresponding row in
    /// `NullRecords`.
    pub fn insert(
        self,
        data: &RawRecordData,
        canonical: &RemoteId,
    ) -> Result<RecordRow<'conn>, rusqlite::Error> {
        debug!("Deleting null records row for `{canonical}`");
        self.tx
            .prepare(sql::delete_null_record())?
            .execute((self.row_id,))?;
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

    /// Delete the row in `NullRecords`.
    pub fn delete(self) -> Result<MissingRow<'conn>, rusqlite::Error> {
        debug!("Deleting cached null for null row '{}'", self.row_id);
        self.tx
            .prepare(sql::delete_cached_null())?
            .execute((self.row_id,))?;
        Ok(MissingRow::new(self.tx))
    }
}

/// Get the most recently attempted time of a [`NullRecordRow`].
#[inline]
pub fn get_null_attempted(row: &NullRecordRow) -> Result<DateTime<Local>, rusqlite::Error> {
    debug!("Getting attempted time for null row '{}'.", row.row_id);
    let NullRowData { attempted, .. } = get_null_row_data(row)?;
    Ok(attempted)
}

/// Get the row data corresponding to a [`NullRecordRow`].
pub fn get_null_row_data(null_row: &NullRecordRow) -> Result<NullRowData, rusqlite::Error> {
    // add_refs_impl(std::iter::once(alias), CitationKeyInsertMode::FailIfExists)
    debug!("Getting null records data for row '{}'.", null_row.row_id);
    let mut record_selector = null_row.tx.prepare_cached(sql::get_cached_null())?;
    let mut record_rows = record_selector.query([null_row.row_id])?;
    record_rows
        .next()?
        .expect("RowId does not exist!")
        .try_into()
}
