use chrono::{DateTime, Local};
use log::debug;

use crate::{
    db::{flatten_constraint_violation, get_row_id, sql, CitationKey, Constraint, RowId},
    Alias, RawRecordData, RemoteId,
};

use super::{DatabaseId, InDatabase, State};

/// An identifier for a row in the `Records` table.
#[derive(Debug)]
pub struct RecordRow(RowId);

impl DatabaseId for RecordRow {}

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

impl InDatabase for RecordRow {
    type Data = RowData;

    const GET_STMT: &str = sql::get_record_data();

    const DELETE_STMT: &str = sql::delete_record_row();

    fn row_id(&self) -> RowId {
        self.0
    }

    fn from_row_id(id: RowId) -> Self {
        Self(id)
    }
}

impl<'conn> State<'conn, RecordRow> {
    /// Copy the [`RowData`] of a row corresponding to a [`RecordRow`] to the `Changelog` table.
    pub fn save_to_changelog(&self) -> Result<(), rusqlite::Error> {
        debug!("Saving row '{}' to Changelog table", self.row_id());
        self.prepare_cached(sql::copy_to_changelog())?
            .execute((self.row_id(),))?;
        Ok(())
    }

    /// Get every key in the `CitationKeys` table which references the [`RecordRow`].
    pub fn get_referencing_keys(&self) -> Result<Vec<String>, rusqlite::Error> {
        debug!("Getting referencing keys for '{}'.", self.row_id());
        let mut selector = self.prepare(sql::get_all_referencing_citation_keys())?;
        let rows = selector.query_map((self.row_id(),), |row| row.get(0))?;
        let mut referencing = Vec::with_capacity(1);
        for name_res in rows {
            referencing.push(name_res?);
        }
        Ok(referencing)
    }

    /// Get the canonical [`RemoteId`].
    #[inline]
    pub fn get_canonical(&self) -> Result<RemoteId, rusqlite::Error> {
        debug!("Getting canonical identifier for '{}'.", self.row_id());
        let RowData { canonical, .. } = self.get_data()?;
        Ok(canonical)
    }

    /// Get last modified time.
    #[inline]
    pub fn last_modified(&self) -> Result<DateTime<Local>, rusqlite::Error> {
        debug!("Getting last modified time for row '{}'.", self.row_id());
        let RowData { modified, .. } = self.get_data()?;
        Ok(modified)
    }

    /// Replace the row data with new data.
    pub fn update_row_data(&self, data: &RawRecordData) -> Result<(), rusqlite::Error> {
        debug!("Updating row data for row '{}'", self.row_id());
        let mut updater = self.prepare(sql::update_cached_data())?;
        updater.execute((self.row_id(), &Local::now(), data.to_byte_repr()))?;
        Ok(())
    }

    /// Add a new alias for this row.
    ///
    /// The return value is `false` if the alias already exists, and otherwise `true`.
    #[inline]
    pub fn add_alias(&self, alias: &Alias) -> Result<bool, rusqlite::Error> {
        self.add_refs_impl(std::iter::once(alias), CitationKeyInsertMode::FailIfExists)
    }

    /// Ensure that the given alias exists for this row.
    ///
    /// If the alias already exists and points to a different row, the canonical id of the other row is returned.
    #[inline]
    pub fn ensure_alias(&self, alias: &Alias) -> Result<Option<RemoteId>, rusqlite::Error> {
        debug!(
            "Ensuring alias '{alias}' refers to row_id '{}'",
            self.row_id()
        );
        match get_row_id(&self.tx, alias)? {
            Some(existing_row_id) => {
                if existing_row_id == self.row_id() {
                    Ok(None)
                } else {
                    let RowData { canonical, .. } = self
                        .tx
                        .prepare_cached(sql::get_record_data())?
                        .query_row([existing_row_id], |row| row.try_into())?;
                    Ok(Some(canonical))
                }
            }
            None => {
                self.prepare(sql::set_citation_key_ignore())?
                    .execute((alias.name(), self.row_id()))?;
                Ok(None)
            }
        }
    }

    /// Insert [`CitationKey`] references for this row.
    ///
    /// The return value is `false` if the insertion failed and `CitationKeyInsertMode` is
    /// `FailIfExists`, and otherwise `true`.
    #[inline]
    pub fn add_refs<'a, R: Iterator<Item = &'a RemoteId>>(
        &self,
        refs: R,
    ) -> Result<bool, rusqlite::Error> {
        self.add_refs_impl(refs, CitationKeyInsertMode::Overwrite)
    }

    /// Insert [`CitationKey`] references for this row.
    ///
    /// The return value is `false` if the insertion failed and `CitationKeyInsertMode` is
    /// `FailIfExists`, and otherwise `true`.
    fn add_refs_impl<'a, K: CitationKey + 'a, R: Iterator<Item = &'a K>>(
        &self,
        refs: R,
        mode: CitationKeyInsertMode,
    ) -> Result<bool, rusqlite::Error> {
        debug!("Inserting references to row_id '{}'", self.row_id());
        for remote_id in refs {
            let stmt = match mode {
                CitationKeyInsertMode::Overwrite => sql::set_citation_key_overwrite(),
                CitationKeyInsertMode::IgnoreIfExists => sql::set_citation_key_ignore(),
                CitationKeyInsertMode::FailIfExists => sql::set_citation_key_fail(),
            };
            let mut key_writer = self.prepare(stmt)?;
            match flatten_constraint_violation(
                key_writer.execute((remote_id.name(), self.row_id())),
            )? {
                Constraint::Satisfied(_) => {}
                Constraint::Violated => return Ok(false),
            }
        }
        Ok(true)
    }
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
