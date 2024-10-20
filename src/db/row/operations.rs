use chrono::{DateTime, Local};
use log::debug;

use super::{DatabaseTransaction, MissingRecordRow, RecordRow, RowData};
use crate::{
    db::{flatten_constraint_violation, sql, Constraint},
    Alias, CitationKey, RawRecordData, RemoteId,
};

/// Get every key in the `CitationKeys` table which references the [`RecordRow`].
pub fn get_referencing_keys(row: &RecordRow) -> Result<Vec<String>, rusqlite::Error> {
    debug!("Getting referencing keys for '{}'.", row.row_id());
    let mut selector = row.prepare(sql::get_all_referencing_citation_keys())?;
    let rows = selector.query_map((row.row_id(),), |row| row.get(0))?;
    let mut referencing = Vec::with_capacity(1);
    for name_res in rows {
        referencing.push(name_res?);
    }
    Ok(referencing)
}

/// Get the canonical [`RemoteId`] corresponding to a [`RecordRow`].
pub fn get_canonical(row: &RecordRow) -> Result<RemoteId, rusqlite::Error> {
    debug!("Getting canonical identifier for '{}'.", row.row_id());
    let RowData { canonical, .. } = get_row_data(row)?;
    Ok(canonical)
}

/// Get the [`RowData`] corresponding to a [`RecordRow`].
#[inline]
pub fn last_modified(row: &RecordRow) -> Result<DateTime<Local>, rusqlite::Error> {
    debug!("Getting data for row '{}'.", row.row_id());
    let RowData { modified, .. } = get_row_data(row)?;
    Ok(modified)
}

/// Get the [`RowData`] corresponding to a [`RecordRow`].
#[inline]
pub fn get_row_data(row: &RecordRow) -> Result<RowData, rusqlite::Error> {
    debug!("Getting data for row '{}'.", row.row_id());
    let mut record_selector = row.prepare_cached(sql::get_cached_data())?;
    let mut record_rows = record_selector.query([row.row_id()])?;
    record_rows
        .next()?
        .expect("RowId does not exist!")
        .try_into()
}

/// Copy the [`RowData`] of a row corresponding to a [`RecordRow`] to the `Changelog` table.
pub fn save_row_to_changelog(row: &RecordRow) -> Result<(), rusqlite::Error> {
    debug!("Saving row '{}' to Changelog table", row.row_id());
    row.prepare_cached(sql::copy_to_changelog())?
        .execute((row.row_id(),))?;
    Ok(())
}

/// Replace the [`RawRecordData`] corresponding to an existing [`RecordRow`] with new data.
pub fn update_row_data(
    data: &RawRecordData,
) -> impl FnOnce(&RecordRow) -> Result<(), rusqlite::Error> + '_ {
    move |row| {
        debug!("Updating row data for row '{}'", row.row_id());
        save_row_to_changelog(row)?;
        let mut updater = row.prepare(sql::update_cached_data())?;
        updater.execute((row.row_id(), &Local::now(), data.to_byte_repr()))?;
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
) -> impl FnOnce(&MissingRecordRow) -> Result<NullRecordsResponse, rusqlite::Error> + '_ {
    move |missing| {
        debug!("Checking null entry for '{remote_id}'");
        let mut null_selector = missing.prepare_cached(sql::get_cached_null())?;
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
) -> impl FnOnce(&MissingRecordRow) -> Result<(), rusqlite::Error> {
    move |missing| {
        let mut setter = missing.prepare_cached(sql::set_cached_null())?;
        let cache_time = Local::now();
        for remote_id in remote_id_iter {
            debug!("Setting null entry for '{remote_id}'");
            setter.execute((remote_id.name(), cache_time))?;
        }

        Ok(())
    }
}

/// Add a new alias to the [`RecordRow`].
///
/// The return value is `false` if the alias already exists, and otherwise `true`.
pub fn add_alias(alias: &Alias) -> impl FnOnce(&RecordRow) -> Result<bool, rusqlite::Error> + '_ {
    add_refs_impl(std::iter::once(alias), CitationKeyInsertMode::FailIfExists)
}

/// Insert [`CitationKey`] references to the row corresponding to a [`RecordRow`].
///
/// The return value is `false` if the insertion failed and `CitationKeyInsertMode` is
/// `FailIfExists`, and otherwise `true`.
pub fn add_refs<'a, R: Iterator<Item = &'a RemoteId>>(
    refs: R,
) -> impl FnOnce(&RecordRow) -> Result<bool, rusqlite::Error> {
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

/// Insert [`CitationKey`] references to the row corresponding to a [`RecordRow`].
///
/// The return value is `false` if the insertion failed and `CitationKeyInsertMode` is
/// `FailIfExists`, and otherwise `true`.
#[inline]
fn add_refs_impl<'a, K: CitationKey + 'a, R: Iterator<Item = &'a K>>(
    refs: R,
    mode: CitationKeyInsertMode,
) -> impl FnOnce(&RecordRow) -> Result<bool, rusqlite::Error> {
    move |row| {
        debug!("Inserting references to row_id '{}'", row.row_id());
        for remote_id in refs {
            let stmt = match mode {
                CitationKeyInsertMode::Overwrite => sql::set_citation_key_overwrite(),
                CitationKeyInsertMode::IgnoreIfExists => sql::set_citation_key_ignore(),
                CitationKeyInsertMode::FailIfExists => sql::set_citation_key_fail(),
            };
            let mut key_writer = row.prepare(stmt)?;
            match flatten_constraint_violation(
                key_writer.execute((remote_id.name(), row.row_id())),
            )? {
                Constraint::Satisfied(_) => {}
                Constraint::Violated => return Ok(false),
            }
        }
        Ok(true)
    }
}
