use chrono::{DateTime, Local};
use serde_bibtex::token::is_entry_key;

use crate::{
    Alias, RawEntryData, RemoteId,
    db::{CitationKey, Constraint, RowId, flatten_constraint_violation, get_row_id, sql},
    entry::EntryData,
    logger::debug,
};

pub trait InRecordsTable {
    fn row_id(&self) -> RowId;
}

impl InRecordsTable for EntryRow {
    fn row_id(&self) -> RowId {
        self.0
    }
}

impl InRecordsTable for RecordRow {
    fn row_id(&self) -> RowId {
        self.0
    }
}

impl InRecordsTable for DeletedRow {
    fn row_id(&self) -> RowId {
        self.0
    }
}

impl<'conn, I: InRecordsTable> State<'conn, I> {
    fn row_id(&self) -> RowId {
        self.id.row_id()
    }

    /// Get every key in the `CitationKeys` table which references the [`RecordRow`].
    pub fn get_referencing_keys(&self) -> Result<Vec<String>, rusqlite::Error> {
        self.get_referencing_keys_impl(Some)
    }

    /// Get every remote id in the `CitationKeys` table which references the [`RecordRow`].
    pub fn get_referencing_remote_ids(&self) -> Result<Vec<RemoteId>, rusqlite::Error> {
        self.get_referencing_keys_impl(RemoteId::from_alias_or_remote_id_unchecked)
    }

    /// Get keys equivalent to a given key that are valid BibTeX citation keys.
    pub fn get_valid_referencing_keys(&self) -> Result<Vec<String>, rusqlite::Error> {
        let mut referencing_keys = self.get_referencing_keys()?;
        referencing_keys.retain(|k| is_entry_key(k));
        Ok(referencing_keys)
    }

    /// Get a transformed version of every key in the `CitationKeys` table which references
    /// the [`RecordRow`] for which the provided `filter_map` does not return `None`.
    fn get_referencing_keys_impl<T, F: FnMut(String) -> Option<T>>(
        &self,
        mut filter_map: F,
    ) -> Result<Vec<T>, rusqlite::Error> {
        debug!("Getting referencing keys for '{}'.", self.row_id());
        let mut selector = self.prepare(sql::get_all_referencing_citation_keys())?;
        let rows = selector.query_map((self.row_id(),), |row| row.get(0))?;
        let mut referencing = Vec::with_capacity(1);
        for name_res in rows {
            if let Some(mapped) = filter_map(name_res?) {
                referencing.push(mapped);
            }
        }
        Ok(referencing)
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

use super::{Missing, State};

#[derive(Debug)]
pub struct RecordRow(pub(in crate::db::state) RowId);

/// The row data associated with a row in the `Records` table. The precise value depends on the
/// `variant` column.
pub enum RecordRowVariant {
    /// Variant `0`, which is regular data
    Entry(RawEntryData),
    /// Variant `1`, which is a row which has been deleted
    Deleted(Option<RemoteId>),
}

pub struct EntryRowData {
    pub data: RawEntryData,
    pub canonical: RemoteId,
    pub modified: DateTime<Local>,
}

pub struct DeletedRowData {
    pub replacement: Option<RemoteId>,
    pub canonical: RemoteId,
    pub modified: DateTime<Local>,
}

pub struct RecordRowData {
    pub variant: RecordRowVariant,
    pub canonical: RemoteId,
    pub modified: DateTime<Local>,
}

impl TryFrom<&rusqlite::Row<'_>> for EntryRowData {
    type Error = rusqlite::Error;

    fn try_from(row: &rusqlite::Row<'_>) -> Result<Self, Self::Error> {
        let data = RawEntryData::from_byte_repr_unchecked(row.get("data")?);
        let canonical = RemoteId::from_string_unchecked(row.get("record_id")?);
        let modified = row.get("modified")?;
        Ok(Self {
            data,
            canonical,
            modified,
        })
    }
}

impl TryFrom<&rusqlite::Row<'_>> for RecordRowData {
    type Error = rusqlite::Error;

    fn try_from(row: &rusqlite::Row<'_>) -> Result<Self, Self::Error> {
        let variant: i64 = row.get("variant")?;
        let data = match variant {
            0 => RecordRowVariant::Entry(RawEntryData::from_byte_repr_unchecked(row.get("data")?)),
            1 => {
                let s: String = row.get("data")?;
                let remote_id = if s.is_empty() {
                    None
                } else {
                    Some(RemoteId::from_string_unchecked(s))
                };
                RecordRowVariant::Deleted(remote_id)
            }
            v => {
                panic!("Corrupted database: 'Records' table contains row with invalid variant {v}")
            }
        };
        let canonical = RemoteId::from_string_unchecked(row.get("record_id")?);
        let modified = row.get("modified")?;
        Ok(Self {
            variant: data,
            canonical,
            modified,
        })
    }
}

#[derive(Debug)]
pub struct DeletedRow(pub(in crate::db::state) RowId);

#[derive(Debug)]
pub struct EntryRow(pub(in crate::db::state) RowId);

impl<'conn> State<'conn, EntryRow> {
    pub fn get_data(&self) -> Result<EntryRowData, rusqlite::Error> {
        debug!(
            "Retrieving 'Records' data associated with row '{}'",
            self.row_id()
        );
        self.prepare_cached(sql::get_record_data())?
            .query_row([self.row_id()], |row| row.try_into())
    }

    pub fn soft_delete(
        self,
        replacement: Option<RemoteId>,
    ) -> Result<State<'conn, DeletedRow>, rusqlite::Error> {
        todo!()
    }

    /// Delete the row.
    pub fn delete(self) -> Result<State<'conn, Missing>, rusqlite::Error> {
        debug!("Deleting row '{}'", self.row_id());
        self.prepare(sql::delete_record_row())?
            .execute((self.row_id(),))?;
        let Self { tx, .. } = self;
        Ok(State::init(tx, Missing {}))
    }

    /// Delete the data associated with the provided citation key and modify the entry in
    /// `CitationKeys` to point to this row. Returns the resulting [`RecordRowData`] if deletion was
    /// successful, and otherwise `None`. Deletion will fail if citation key is not present in the
    /// database.
    ///
    /// The `missing_cb` is called if the provided citation key is not present in the database.
    /// Citation keys which are equivalent to the row are skipped.
    pub fn absorb<K: CitationKey>(
        &self,
        record_id: &K,
        missing_cb: impl FnOnce(),
    ) -> Result<Option<RecordRowData>, rusqlite::Error> {
        Ok(match get_row_id(&self.tx, record_id)? {
            Some(row_id) if row_id != self.row_id() => {
                // update rows in CitationKeys
                self.prepare(sql::redirect_citation_key())?
                    .execute((self.row_id(), row_id))?;

                // TODO: come up with a better abstraction that allows nested `Row`s.

                // get the row data
                let row_data: RecordRowData = self
                    .prepare_cached(sql::get_record_data())?
                    .query_row([row_id], |row| row.try_into())?;

                // FIXME: previously copied to changelog here

                // delete the row
                self.prepare_cached(sql::delete_record_row())?
                    .execute([row_id])?;

                Some(row_data)
            }
            None => {
                missing_cb();
                None
            }
            _ => None,
        })
    }

    /// Get the canonical [`RemoteId`].
    #[inline]
    pub fn get_canonical(&self) -> Result<RemoteId, rusqlite::Error> {
        debug!("Getting canonical identifier for '{}'.", self.row_id());
        let EntryRowData { canonical, .. } = self.get_data()?;
        Ok(canonical)
    }

    /// Get last modified time.
    #[inline]
    pub fn last_modified(&self) -> Result<DateTime<Local>, rusqlite::Error> {
        debug!("Getting last modified time for row '{}'.", self.row_id());
        let EntryRowData { modified, .. } = self.get_data()?;
        Ok(modified)
    }

    /// A convenience wrapper around [`update`](Self::update) which first converts any type which
    /// implements [`EntryData`] into a [`RawEntryData`].
    pub fn update_entry_data<D: EntryData>(&self, data: &D) -> Result<(), rusqlite::Error> {
        self.update(&RawEntryData::from_entry_data(data))
    }

    /// Replace the row data with new data.
    pub fn update(&self, data: &RawEntryData) -> Result<(), rusqlite::Error> {
        debug!("Updating row data for row '{}'", self.row_id());
        let mut updater = self.prepare(sql::update_cached_data())?;
        updater.execute((self.row_id(), Local::now(), data.to_byte_repr()))?;
        Ok(())
    }

    /// Change the canonical id of the row.
    ///
    /// Returns `false` if the new canonical id already exists, and `true` otherwise.
    pub fn change_canonical_id(&self, new_id: &RemoteId) -> Result<bool, rusqlite::Error> {
        let old_id = self.get_canonical()?;
        debug!(
            "Changing the canonical id for row '{}' from '{old_id}' to '{new_id}'",
            self.row_id()
        );
        let result = self.prepare(sql::update_canonical_id())?.execute((
            self.row_id(),
            Local::now(),
            new_id.to_string(),
        ));
        if let Constraint::Violated = flatten_constraint_violation(result)? {
            return Ok(false);
        }
        self.add_refs_impl(std::iter::once(new_id), CitationKeyInsertMode::FailIfExists)?;
        self.prepare(sql::delete_citation_key())?
            .execute((old_id.name(),))?;
        Ok(true)
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
                    let RecordRowData { canonical, .. } = self
                        .tx
                        .prepare_cached(sql::get_record_data())?
                        .query_row([existing_row_id], |row| row.try_into())?;
                    Ok(Some(canonical))
                }
            }
            None => {
                self.prepare(sql::set_citation_key_fail())?
                    .execute((alias.name(), self.row_id()))?;
                Ok(None)
            }
        }
    }

    /// Check if the given alias exists and points to this row, and delete the alias if it does.
    #[inline]
    pub fn delete_alias_if_associated(&self, alias: &Alias) -> Result<(), rusqlite::Error> {
        debug!(
            "Checking if alias '{alias}' refers to row_id '{}' and deleting the alias if yes",
            self.row_id()
        );
        if let Some(row_id) = get_row_id(&self.tx, alias)?
            && row_id == self.row_id()
        {
            self.prepare(sql::delete_citation_key())?
                .execute((alias.name(),))?;
        }
        Ok(())
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
}

pub enum ResolvedRecordRowState<'conn> {
    Exists(EntryRowData, State<'conn, EntryRow>),
    Deleted(DeletedRowData, State<'conn, DeletedRow>),
}

impl<'conn> State<'conn, RecordRow> {
    /// Add a new alias for this row.
    ///
    /// The return value is `false` if the alias already exists, and otherwise `true`.
    #[inline]
    pub fn add_alias(&self, alias: &Alias) -> Result<bool, rusqlite::Error> {
        self.add_refs_impl(std::iter::once(alias), CitationKeyInsertMode::FailIfExists)
    }

    pub fn get_data(&self) -> Result<RecordRowData, rusqlite::Error> {
        debug!(
            "Retrieving 'Records' data associated with row '{}'",
            self.row_id()
        );
        self.prepare_cached(sql::get_record_data())?
            .query_row([self.row_id()], |row| row.try_into())
    }

    pub fn resolve(self) -> Result<ResolvedRecordRowState<'conn>, rusqlite::Error> {
        let RecordRowData {
            variant,
            modified,
            canonical,
        } = self.get_data()?;

        let row_id = self.row_id();

        Ok(match variant {
            RecordRowVariant::Entry(data) => ResolvedRecordRowState::Exists(
                EntryRowData {
                    data,
                    modified,
                    canonical,
                },
                State::init(self.tx, EntryRow(row_id)),
            ),
            RecordRowVariant::Deleted(into) => ResolvedRecordRowState::Deleted(
                DeletedRowData {
                    replacement: into,
                    modified,
                    canonical,
                },
                State::init(self.tx, DeletedRow(row_id)),
            ),
        })
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
