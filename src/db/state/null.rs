use chrono::{DateTime, Local};

use super::{Missing, State};
use crate::{
    db::{RowId, sql},
    logger::debug,
};

/// An identifier for a row in the `NullRecords` table.
#[derive(Debug)]
pub struct NullRecordRow(pub(in crate::db::state) RowId);

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

impl<'conn> State<'conn, NullRecordRow> {
    fn row_id(&self) -> RowId {
        self.id.0
    }

    /// Delete the null record.
    pub fn delete(self) -> Result<State<'conn, Missing>, rusqlite::Error> {
        debug!("Deleting 'NullRecords' row '{}'", self.row_id());
        self.prepare(sql::delete_null_record_row())?
            .execute((self.row_id(),))?;
        let Self { tx, .. } = self;
        Ok(State::init(tx, Missing {}))
    }

    /// Get the data associated with the row.
    pub fn get_data(&self) -> Result<NullRowData, rusqlite::Error> {
        debug!("Retrieving 'NullRecords' row '{}'", self.row_id());
        self.prepare_cached(sql::get_null_record_data())?
            .query_row([self.row_id()], |row| row.try_into())
    }

    /// Get the time when the null was cached.
    pub fn get_null_attempted(&self) -> Result<DateTime<Local>, rusqlite::Error> {
        debug!("Getting attempted time for null row '{}'.", self.row_id());
        let NullRowData { attempted, .. } = self.get_data()?;
        Ok(attempted)
    }
}
