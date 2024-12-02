use chrono::{DateTime, Local};

use super::{DatabaseId, InDatabase, State};
use crate::{
    db::{sql, RowId},
    logger::debug,
};

/// An identifier for a row in the `NullRecords` table.
#[derive(Debug)]
pub struct NullRecordRow(RowId);

impl DatabaseId for NullRecordRow {}

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

impl InDatabase for NullRecordRow {
    type Data = NullRowData;

    const GET_STMT: &str = sql::get_null_record_data();

    const DELETE_STMT: &str = sql::delete_null_record_row();

    fn row_id(&self) -> RowId {
        self.0
    }

    fn from_row_id(id: RowId) -> Self {
        Self(id)
    }
}

impl State<'_, NullRecordRow> {
    /// Get the time when the null was cached.
    pub fn get_null_attempted(&self) -> Result<DateTime<Local>, rusqlite::Error> {
        debug!("Getting attempted time for null row '{}'.", self.row_id());
        let NullRowData { attempted, .. } = self.get_data()?;
        Ok(attempted)
    }
}
