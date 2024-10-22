use chrono::Local;
use log::debug;
use rusqlite::Transaction;

use super::{
    add_refs, transaction::tx_impl, DatabaseState, NullRecordRow, RecordRow, RemoteIdState,
};
use crate::{
    db::{sql, CitationKey},
    RawRecordData, RemoteId,
};

/// The database state when:
/// 1. There is no row in `Records`.
/// 2. There is no row in `NullRecords`.
#[derive(Debug)]
pub struct MissingRow<'conn> {
    tx: Transaction<'conn>,
}

tx_impl!(MissingRow);

impl<'conn> MissingRow<'conn> {
    /// Initialize a new [`MissingRow`].
    pub(super) fn new(tx: Transaction<'conn>) -> Self {
        Self { tx }
    }

    /// Set a null row, converting into a [`NullRecordRow`].
    pub fn set_null(self, remote_id: &RemoteId) -> Result<NullRecordRow<'conn>, rusqlite::Error> {
        {
            let mut setter = self.tx.prepare_cached(sql::set_cached_null())?;
            let cache_time = Local::now();
            setter.execute((remote_id.name(), cache_time))?;
        }
        let row_id = self.tx.last_insert_rowid();

        Ok(NullRecordRow::new(self.tx, row_id))
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

    /// Reset the row, clearing any internal data but preserving the transaction.
    pub fn reset(self, remote_id: &RemoteId) -> Result<RemoteIdState<'conn>, rusqlite::Error> {
        RemoteIdState::determine(self.tx, remote_id)
    }
}
