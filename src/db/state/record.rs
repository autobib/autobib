use std::cmp::Reverse;

use chrono::{DateTime, Local};
use rusqlite::{OptionalExtension, Row};

use crate::{
    Alias, RawEntryData, RemoteId,
    db::{Constraint, Identifier, flatten_constraint_violation, get_row_id},
    logger::{debug, info},
};

use super::{IsMissing, State, Transaction, version::RevisionId};

/// Any state which represents a row in the 'Records' table.
pub trait InRecordsTable {
    /// The data associated with the row.
    type Data: AsRecordRowData + FromBytesAndVariant;

    /// Convert to a row id.
    fn row_id(&self) -> i64;
}

/// Any state which represents a row in the 'Records' table which is not void.
pub trait NotVoid: InRecordsTable {}

/// Any state which represents a row in the 'Records' table which is not an entry.
pub trait NotEntry: InRecordsTable {}

/// A wrapper trait for data which can be read from the 'data' and 'variant' columns of the
/// 'Records' table.
pub trait FromBytesAndVariant: Sized {
    fn from_bytes_and_variant(bytes: Vec<u8>, variant: i64) -> Self;
}

/// The data for a row in the 'Records' table, not including information about the parents.
#[derive(Debug)]
pub struct RecordRow<D, S = String> {
    /// The associated data.
    pub data: D,
    /// The canonical identifier.
    pub canonical: RemoteId<S>,
    /// When the record was modified.
    pub modified: DateTime<Local>,
}

impl<D: FromBytesAndVariant> RecordRow<D> {
    /// Load from a row in the 'Records' table. The query which produced the row must contain the following keyds:
    ///
    /// - `record_id`,
    /// - `modified`
    /// - `data`
    /// - `variant`
    pub(in crate::db) fn from_row_unchecked(row: &Row<'_>) -> Self {
        let data = D::from_bytes_and_variant(row.get_unwrap("data"), row.get_unwrap("variant"));
        let canonical = RemoteId::from_string_unchecked(row.get_unwrap("record_id"));
        let modified = row.get_unwrap("modified");

        Self {
            data,
            modified,
            canonical,
        }
    }

    /// Load from a row id, which the caller promises is a valid row ID in the 'Records' table and
    /// moreover has type `D`.
    pub(super) fn load_unchecked(tx: &Transaction<'_>, row_id: i64) -> rusqlite::Result<Self> {
        tx.prepare_cached("SELECT record_id, modified, data, variant FROM Records WHERE key = ?1")?
            .query_row((row_id,), |row| Ok(Self::from_row_unchecked(row)))
    }
}

/// The data for a row in the 'Records' table, also including information about the parents.
pub struct CompleteRecordRow<D> {
    pub row: RecordRow<D>,
    pub(super) parent: Option<i64>,
}

impl<D: FromBytesAndVariant> CompleteRecordRow<D> {
    pub(super) fn from_row_unchecked(row: &Row<'_>) -> Self {
        let parent = row.get_unwrap("parent_key");
        let row = RecordRow::from_row_unchecked(row);

        Self { row, parent }
    }

    pub(super) fn load_unchecked(tx: &Transaction<'_>, row_id: i64) -> rusqlite::Result<Self> {
        tx.prepare_cached(
            "SELECT record_id, modified, data, variant, parent_key FROM Records WHERE key = ?1",
        )?
        .query_row((row_id,), |row| Ok(Self::from_row_unchecked(row)))
    }
}

trait FromRowId: InRecordsTable {
    /// Construct from a row id.
    fn from_row_id(row_id: i64) -> Self;
}

macro_rules! impl_record_key {
    ($v:ident, $data:ty) => {
        impl std::fmt::Display for $v {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "#{:x}", self.0)
            }
        }

        impl InRecordsTable for $v {
            type Data = $data;

            fn row_id(&self) -> i64 {
                self.0
            }
        }

        impl FromRowId for $v {
            fn from_row_id(id: i64) -> Self {
                $v(id)
            }
        }
    };
}

/// The `key` of a row in the 'Records' table which is either an `entry` or `deleted`.
#[derive(Debug)]
pub struct IsArbitrary(pub(super) i64);

/// The row data associated with a row in the `Records` table. The precise value depends on the
/// `variant` column.
#[derive(Debug)]
pub enum ArbitraryData {
    /// Entry data.
    Entry(RawEntryData),
    /// Deleted data.
    Deleted(Option<RemoteId>),
    /// Void data.
    Void,
}

impl FromBytesAndVariant for ArbitraryData {
    fn from_bytes_and_variant(bytes: Vec<u8>, variant: i64) -> Self {
        match variant {
            0 => Self::Entry(RawEntryData::from_bytes_and_variant(bytes, variant)),
            1 => Self::Deleted(Option::<RemoteId>::from_bytes_and_variant(bytes, variant)),
            2 => Self::Void,
            _ => panic!("Unexpected 'Records' table row variant: expected entry or deleted data."),
        }
    }
}

impl_record_key!(IsArbitrary, ArbitraryData);

/// The `key` of a row in the 'Records' table which is either an `entry` or `deleted`.
#[derive(Debug)]
pub struct IsEntryOrDeleted(pub(super) i64);

/// The row data associated with a row in the `Records` table. The precise value depends on the
/// `variant` column.
#[derive(Debug)]
pub enum EntryOrDeletedData {
    /// Entry data.
    Entry(RawEntryData),
    /// Deleted data.
    Deleted(Option<RemoteId>),
}

impl FromBytesAndVariant for EntryOrDeletedData {
    fn from_bytes_and_variant(bytes: Vec<u8>, variant: i64) -> Self {
        match variant {
            0 => Self::Entry(RawEntryData::from_bytes_and_variant(bytes, variant)),
            1 => Self::Deleted(Option::<RemoteId>::from_bytes_and_variant(bytes, variant)),
            _ => panic!("Unexpected 'Records' table row variant: expected entry or deleted data."),
        }
    }
}

impl NotVoid for IsEntryOrDeleted {}

impl_record_key!(IsEntryOrDeleted, EntryOrDeletedData);

/// An entry in the 'Records' table.
#[derive(Debug)]
pub struct IsEntry(pub(super) i64);

impl FromBytesAndVariant for RawEntryData {
    fn from_bytes_and_variant(bytes: Vec<u8>, variant: i64) -> Self {
        assert!(
            variant == 0,
            "Unexpected 'Records' table row variant: expected entry data."
        );
        Self::from_byte_repr_unchecked(bytes)
    }
}

impl NotVoid for IsEntry {}

impl_record_key!(IsEntry, RawEntryData);

/// A deletion marker in the 'Records' table.
#[derive(Debug)]
pub struct IsDeleted(i64);

impl FromBytesAndVariant for Option<RemoteId> {
    fn from_bytes_and_variant(bytes: Vec<u8>, variant: i64) -> Self {
        assert!(
            variant == 1,
            "Unexpected 'Records' table row variant: expected deletion marker."
        );
        if bytes.is_empty() {
            None
        } else {
            Some(RemoteId::from_string_unchecked(bytes.try_into().expect(
                "Invalid database: 'data' column for deleted row contains non-UTF8 blob data.",
            )))
        }
    }
}

impl NotVoid for IsDeleted {}
impl NotEntry for IsDeleted {}

impl_record_key!(IsDeleted, Option<RemoteId>);

/// The 'void' root node in the 'Records' table.
///
/// In order to save database state, this type is typically not stored at all,
/// is created when required to undo into the deleted state which precedes all record of the data.
#[derive(Debug)]
pub struct IsVoid(pub(super) i64);

impl FromBytesAndVariant for () {
    fn from_bytes_and_variant(_: Vec<u8>, variant: i64) -> Self {
        assert!(
            variant == 2,
            "Unexpected 'Records' table row variant: expected void marker"
        );
    }
}

impl NotEntry for IsVoid {}

impl_record_key!(IsVoid, ());

/// A row in the 'Records' table, disambiguated based on what type of row it is.
pub enum DisambiguatedRecordRow<'conn> {
    Entry(RecordRow<RawEntryData>, State<'conn, IsEntry>),
    Deleted(RecordRow<Option<RemoteId>>, State<'conn, IsDeleted>),
    Void(RecordRow<()>, State<'conn, IsVoid>),
}

impl<'conn> DisambiguatedRecordRow<'conn> {
    pub fn forget(self) -> (RecordRow<ArbitraryData>, State<'conn, IsArbitrary>) {
        match self {
            Self::Entry(data, state) => (data.into(), state.forget()),
            Self::Deleted(data, state) => (data.into(), state.forget()),
            Self::Void(data, state) => (data.into(), state.forget()),
        }
    }
}

/// Types which can be written as the 'data' and 'variant' column in the 'Records' table.
pub trait AsRecordRowData: Sized {
    fn data_blob(&self) -> &[u8];

    fn variant(&self) -> i64;
}

impl AsRecordRowData for RawEntryData {
    fn data_blob(&self) -> &[u8] {
        self.to_byte_repr()
    }

    fn variant(&self) -> i64 {
        0
    }
}

impl AsRecordRowData for Option<RemoteId> {
    fn data_blob(&self) -> &[u8] {
        self.as_ref().map_or(b"", |r| r.name().as_bytes())
    }

    fn variant(&self) -> i64 {
        1
    }
}

impl AsRecordRowData for EntryOrDeletedData {
    fn data_blob(&self) -> &[u8] {
        match self {
            Self::Entry(raw_entry_data) => raw_entry_data.data_blob(),
            Self::Deleted(remote_id) => remote_id.data_blob(),
        }
    }

    fn variant(&self) -> i64 {
        match self {
            Self::Entry(raw_entry_data) => raw_entry_data.variant(),
            Self::Deleted(remote_id) => remote_id.variant(),
        }
    }
}

impl AsRecordRowData for ArbitraryData {
    fn data_blob(&self) -> &[u8] {
        match self {
            Self::Entry(raw_entry_data) => raw_entry_data.data_blob(),
            Self::Deleted(remote_id) => remote_id.data_blob(),
            Self::Void => ().data_blob(),
        }
    }

    fn variant(&self) -> i64 {
        match self {
            Self::Entry(raw_entry_data) => raw_entry_data.variant(),
            Self::Deleted(remote_id) => remote_id.variant(),
            Self::Void => ().variant(),
        }
    }
}

impl AsRecordRowData for () {
    fn data_blob(&self) -> &[u8] {
        &[]
    }

    fn variant(&self) -> i64 {
        2
    }
}

/// Get the canonical identifier.
fn get_canonical(tx: &Transaction, row_id: i64) -> rusqlite::Result<RemoteId> {
    tx.prepare_cached("SELECT record_id FROM Records WHERE key = ?1")?
        .query_row([row_id], |row| {
            row.get("record_id").map(RemoteId::from_string_unchecked)
        })
}

/// Get the last modified time.
fn get_last_modified(tx: &Transaction, row_id: i64) -> rusqlite::Result<DateTime<Local>> {
    tx.prepare_cached("SELECT modified FROM Records WHERE key = ?1")?
        .query_row([row_id], |row| row.get("modified"))
}

/// The result after applying a movement command.
pub enum RecordRowMoveResult<'conn, N, O, E> {
    /// The movement command succeeded.
    Updated(State<'conn, N>),
    /// The movement command failed, so the original row is returned along with some error context.
    Unchanged(State<'conn, O>, E),
}

impl<'conn, N, O: InRecordsTable, E> RecordRowMoveResult<'conn, N, O, E> {
    fn from_rowid(
        original: State<'conn, O>,
        candidate: Result<i64, E>,
    ) -> Result<Self, rusqlite::Error>
    where
        N: FromRowId,
    {
        match candidate {
            Ok(row_id) => original.transmute(row_id).map(RecordRowMoveResult::Updated),
            Err(e) => Ok(RecordRowMoveResult::Unchanged(original, e)),
        }
    }
}

pub enum SetActiveError {
    RowIdUndefined,
    DifferentCanonical(RemoteId),
}

impl<'conn, I: InRecordsTable> State<'conn, I> {
    pub(in crate::db) fn row_id(&self) -> i64 {
        self.id.row_id()
    }

    /// Unchecked conversion with a new row id of any type, updating the rows in the Identifiers table.
    fn transmute<N: FromRowId>(self, new_row_id: i64) -> rusqlite::Result<State<'conn, N>> {
        self.update_identifier_lookup(new_row_id)?;
        Ok(State::init(self.tx, N::from_row_id(new_row_id)))
    }

    /// Obtain the data for this row.
    pub fn get_data(&self) -> rusqlite::Result<RecordRow<I::Data>> {
        debug!(
            "Retrieving 'Records' data associated with row '{}'",
            self.row_id()
        );
        RecordRow::load_unchecked(&self.tx, self.row_id())
    }

    /// Get the canonical [`RemoteId`].
    #[inline]
    pub fn canonical(&self) -> Result<RemoteId, rusqlite::Error> {
        debug!("Getting canonical identifier for '{}'.", self.row_id());
        get_canonical(&self.tx, self.row_id())
    }

    /// Get the hexadecimal revision of the active row.
    pub fn rev(&self) -> String {
        format!("{:0>4x}", self.row_id())
    }

    /// Get last modified time.
    #[inline]
    pub fn last_modified(&self) -> Result<DateTime<Local>, rusqlite::Error> {
        debug!("Getting last modified time for row '{}'.", self.row_id());
        get_last_modified(&self.tx, self.row_id())
    }

    /// Obtain the complete data for this row.
    pub fn get_complete_data(&self) -> rusqlite::Result<CompleteRecordRow<I::Data>> {
        debug!(
            "Retrieving 'Records' data associated with row '{}'",
            self.row_id()
        );
        CompleteRecordRow::load_unchecked(&self.tx, self.row_id())
    }

    /// Forget the specific type of row that this is.
    pub fn forget(self) -> State<'conn, IsArbitrary> {
        let row_id = self.row_id();
        State::init(self.tx, IsArbitrary(row_id))
    }

    /// Update the active row to a specific revision.
    ///
    /// If the row-id does not correspond to a row in the 'Records' table with a canonical id which is the same
    /// as the canonical id of this row, this returns an error.
    pub fn set_active(
        self,
        RevisionId(row_id): RevisionId,
    ) -> Result<RecordRowMoveResult<'conn, IsArbitrary, I, SetActiveError>, rusqlite::Error> {
        debug!(
            "Updating the active row for '{}' to '{}'.",
            self.row_id(),
            row_id
        );
        let self_canonical = self.canonical()?;

        // check if the row id corresponds to a row in the records table, and moreover that the
        // corresonding canonical id is the same
        let row_id_or_err = match get_canonical(&self.tx, row_id).optional()? {
            Some(target_canonical) if target_canonical == self_canonical => Ok(row_id),
            Some(other_canonical) => Err(SetActiveError::DifferentCanonical(other_canonical)),
            None => Err(SetActiveError::RowIdUndefined),
        };

        RecordRowMoveResult::from_rowid(self, row_id_or_err)
    }

    /// Repeatedly undo until arriving at a first state precedes the provided time.
    pub fn rewind(self, before: DateTime<Local>) -> rusqlite::Result<State<'conn, IsArbitrary>> {
        debug!(
            "Rewinding row id '{}' to how it looked at {}",
            self.row_id(),
            before
        );
        let canonical = self.canonical()?;
        let new_id = create_rewind_target(&self.tx, canonical.name(), before)?;
        self.transmute(new_id)
    }

    /// Update the 'Identifiers' table by setting any rows which reference the current row to
    /// reference a new row id instead.
    fn update_identifier_lookup(&self, new_key: i64) -> Result<usize, rusqlite::Error> {
        self.prepare("UPDATE Identifiers SET record_key = ?1 WHERE record_key = ?2")?
            .execute((new_key, self.row_id()))
    }

    /// Hard delete the row. This deletes every entry in the 'Records' with the same canonical
    /// identifier as the current row.
    pub fn hard_delete(self) -> Result<State<'conn, IsMissing>, rusqlite::Error> {
        debug!(
            "Permanently deleting all rows in the edit-tree associated with the id '{}'",
            self.row_id()
        );
        self.prepare(
            "DELETE FROM Records WHERE record_id = (SELECT record_id FROM Records WHERE key = ?1);",
        )?
        .execute((self.row_id(),))?;

        Ok(State::init(self.tx, IsMissing))
    }

    /// Get every key in the `Identifiers` table which references this row.
    pub fn referencing_keys(&self) -> Result<Vec<String>, rusqlite::Error> {
        self.referencing_keys_impl(Some)
    }

    /// Get every remote id in the `Identifiers` table which references this row.
    pub fn referencing_remote_ids(&self) -> Result<Vec<RemoteId>, rusqlite::Error> {
        self.referencing_keys_impl(RemoteId::from_alias_or_remote_id_unchecked)
    }

    /// Get a transformed version of every key in the `Identifiers` table which references
    /// the current row for which the provided `filter_map` does not return `None`.
    fn referencing_keys_impl<T, F: FnMut(String) -> Option<T>>(
        &self,
        mut filter_map: F,
    ) -> Result<Vec<T>, rusqlite::Error> {
        debug!("Getting referencing keys for '{}'.", self.row_id());
        let mut selector = self.prepare("SELECT name FROM Identifiers WHERE record_key = ?1")?;
        let rows = selector.query_map((self.row_id(),), |row| row.get(0))?;
        let mut referencing = Vec::with_capacity(1);
        for name_res in rows {
            if let Some(mapped) = filter_map(name_res?) {
                referencing.push(mapped);
            }
        }
        Ok(referencing)
    }

    /// Insert [`RemoteId`] references for this row.
    ///
    /// The return value is `false` if the insertion failed and `IdentifierInsertMode` is
    /// `FailIfExists`, and otherwise `true`.
    #[inline]
    pub fn add_refs<'a, R: Iterator<Item = &'a RemoteId>>(
        &self,
        refs: R,
    ) -> Result<bool, rusqlite::Error> {
        self.add_refs_impl(refs, IdentifierInsertMode::Overwrite)
    }

    /// Insert [`Identifier`] references for this row.
    ///
    /// The return value is `false` if the insertion failed and `IdentifierInsertMode` is
    /// `FailIfExists`, and otherwise `true`.
    fn add_refs_impl<'a, K: Identifier + 'a, R: Iterator<Item = &'a K>>(
        &self,
        refs: R,
        mode: IdentifierInsertMode,
    ) -> Result<bool, rusqlite::Error> {
        debug!("Inserting references to row_id '{}'", self.row_id());
        for remote_id in refs {
            let stmt = match mode {
                IdentifierInsertMode::Overwrite => {
                    "INSERT OR REPLACE INTO Identifiers (name, record_key) values (?1, ?2)"
                }
                IdentifierInsertMode::IgnoreIfExists => {
                    "INSERT OR IGNORE INTO Identifiers (name, record_key) values (?1, ?2)"
                }
                IdentifierInsertMode::FailIfExists => {
                    "INSERT INTO Identifiers (name, record_key) values (?1, ?2)"
                }
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

    /// Insert a new row with data, adding the previous row as the parent.
    fn replace_impl<R: AsRecordRowData>(&self, data: &R) -> Result<i64, rusqlite::Error> {
        let existing = self.get_complete_data()?;

        // insert a new row into Records containing:
        //
        // - the previous value of 'record_id'
        // - the new data
        // - the current timestamp
        // - the correct variant
        // - the key of the row being replaced, in parent_key
        //
        // the remaining fields use their default values
        let new_key: i64 = self.prepare("INSERT INTO Records (record_id, data, modified, variant, parent_key) VALUES (?1, ?2, ?3, ?4, ?5) RETURNING key")?
            .query_row((existing.row.canonical.name(), data.data_blob(), Local::now(), data.variant(), self.row_id()), |row| row.get(0))?;

        self.update_identifier_lookup(new_key)?;

        Ok(new_key)
    }

    /// Perform a redo operation from an arbitrary state.
    fn redo_unchecked(
        self,
        idx: isize,
    ) -> Result<RecordRowMoveResult<'conn, IsArbitrary, I, RedoError>, rusqlite::Error> {
        let version = self.current()?;
        let mut children = Vec::new();
        version.map_children(|data, row_id| {
            children.push((data.row.modified, row_id));
            Ok(())
        })?;

        if idx >= 0 {
            children.sort_unstable_by_key(|c| c.0);
            RecordRowMoveResult::from_rowid(
                self,
                children
                    .get(idx.abs_diff(0))
                    .map(|c| c.1)
                    .ok_or(RedoError::OutOfBounds(children.len())),
            )
        } else {
            children.sort_unstable_by_key(|c| Reverse(c.0));
            RecordRowMoveResult::from_rowid(
                self,
                children
                    .get(idx.abs_diff(-1))
                    .map(|c| c.1)
                    .ok_or(RedoError::OutOfBounds(children.len())),
            )
        }
    }
}

/// A description of the state which prevented an undo operation from completing.
pub enum UndoError {
    /// The parent is an entry.
    ParentEntry,
    /// The parent is a deletion marker.
    ParentDeleted,
    /// The parent is void, and it exists.
    ParentVoidExists,
    /// The parent void is missing.
    ParentVoidMissing,
}

impl<'conn, I: NotVoid> State<'conn, I> {
    /// Update the active row to be the parent of this row, if it exists and is an entry.
    pub fn undo(self) -> rusqlite::Result<RecordRowMoveResult<'conn, IsEntry, I, UndoError>> {
        let row_id_or_err = match self.current()?.parent()? {
            Some(parent) => match parent.row.data {
                ArbitraryData::Entry(_) => Ok(parent.row_id),
                ArbitraryData::Deleted(_) => Err(UndoError::ParentDeleted),
                ArbitraryData::Void => Err(UndoError::ParentVoidExists),
            },
            None => Err(UndoError::ParentVoidMissing),
        };

        RecordRowMoveResult::from_rowid(self, row_id_or_err)
    }

    /// Update the active row to be the parent of this row, if it exists and is deleted.
    pub fn undo_delete(
        self,
    ) -> rusqlite::Result<RecordRowMoveResult<'conn, IsDeleted, I, UndoError>> {
        let row_id_or_err = match self.current()?.parent()? {
            Some(parent) => match parent.row.data {
                ArbitraryData::Entry(_) => Err(UndoError::ParentEntry),
                ArbitraryData::Deleted(_) => Ok(parent.row_id),
                ArbitraryData::Void => Err(UndoError::ParentVoidExists),
            },
            None => Err(UndoError::ParentVoidMissing),
        };

        RecordRowMoveResult::from_rowid(self, row_id_or_err)
    }

    /// Void this record row.
    pub fn void(self) -> rusqlite::Result<State<'conn, IsVoid>> {
        let root = self.current()?.root(true)?;
        let root_row_id = root.row_id;

        let new_row_id = match root.row.data {
            ArbitraryData::Deleted(_) | ArbitraryData::Entry(_) => {
                create_void_parent(root.tx, root.row_id, root.row.canonical.name())?
            }
            ArbitraryData::Void => root_row_id,
        };
        self.update_identifier_lookup(new_row_id)?;
        Ok(State::init(self.tx, IsVoid(new_row_id)))
    }
}

/// Returns a row which can be the target to rewind to.
///
/// If a row exists which has modification time before `before`, this row is returned. Otherwise,
/// a new void root is created, and the row id is returned.
pub(in crate::db) fn create_rewind_target(
    tx: &Transaction<'_>,
    canonical: &str,
    before: DateTime<Local>,
) -> rusqlite::Result<i64> {
    // first, try to find a candidate vertex to swap to
    let id_opt: Option<i64> = tx.prepare("SELECT key FROM Records WHERE record_id = ?1 AND modified <= ?2 ORDER BY modified DESC LIMIT 1")?
            .query_row((canonical, before), |row| row.get(0)).optional()?;

    Ok(if let Some(id) = id_opt {
        id
    } else {
        // if no candidate exists, this means the modified time is > `before` on every entry in
        // canonical, so we find the root and add the void vertex before it
        let root_row_id: i64 = tx
            .prepare("SELECT key FROM Records WHERE record_id = ?1 AND parent_key IS NULL")?
            .query_row([canonical], |row| row.get(0))?;
        create_void_parent(tx, root_row_id, canonical)?
    })
}

/// Create a parent to this row which is a void record.
fn create_void_parent(
    tx: &Transaction<'_>,
    root_row_id: i64,
    canonical: &str,
) -> rusqlite::Result<i64> {
    // create the void root
    let new_row_id: i64 = tx.prepare("INSERT INTO Records (record_id, data, modified, variant) VALUES (?1, ?2, ?3, ?4) RETURNING key")?
            .query_row((canonical, ().data_blob(), DateTime::<Local>::MIN_UTC, ().variant()), |row| row.get(0))?;

    // update the non-void root to reference the parent
    tx.prepare("UPDATE Records SET parent_key = ?1 WHERE key = ?2")?
        .execute((Some(new_row_id), root_row_id))?;

    Ok(new_row_id)
}

impl<'conn, I: NotEntry> State<'conn, I> {
    pub fn redo_deletion(
        self,
        index: isize,
    ) -> Result<RecordRowMoveResult<'conn, IsArbitrary, I, RedoError>, rusqlite::Error> {
        self.redo_unchecked(index)
    }
}

pub enum RedoError {
    OutOfBounds(usize),
    ChildNotUnique(usize),
}

impl<'conn> State<'conn, IsEntry> {
    /// Update the active row to be a child of this row.
    ///
    /// If `index` is none and there is a unique child, this method will succeed. Otherwise,
    /// attempt to set to the `nth` child, ordered from first to last, where negative indices count
    /// backwards.
    ///
    /// The returned index on error is the number of children.
    pub fn redo(
        self,
        index: isize,
    ) -> Result<RecordRowMoveResult<'conn, IsArbitrary, IsEntry, RedoError>, rusqlite::Error> {
        self.redo_unchecked(index)
    }

    /// Soft-delete this row, replacing it with the candidate canonical identifier if the
    /// identifier exists in the record database.
    pub fn update_canonical(
        self,
        candidate: &RemoteId,
        update_aliases: bool,
    ) -> rusqlite::Result<RecordRowMoveResult<'conn, IsDeleted, IsEntry, bool>> {
        let replacement: Option<i64> = self
            .tx
            .prepare("SELECT record_key FROM Identifiers WHERE name = ?1")?
            .query_row([candidate.name()], |row| row.get("record_key"))
            .optional()?;

        match replacement {
            None => Ok(RecordRowMoveResult::Unchanged(self, false)),
            Some(row_id) => {
                if row_id == self.row_id() {
                    Ok(RecordRowMoveResult::Unchanged(self, true))
                } else {
                    let repl: String = self
                        .tx
                        .prepare("SELECT record_id FROM Records WHERE key = ?1")?
                        .query_row([row_id], |row| row.get("record_id"))?;
                    let remote_id = RemoteId::from_string_unchecked(repl);
                    info!("Replacing row with new canonical id '{remote_id}'");
                    let deleted = self.soft_delete(&Some(remote_id), update_aliases)?;
                    Ok(RecordRowMoveResult::Updated(deleted))
                }
            }
        }
    }
}

impl<D> RecordRow<D> {
    /// Convert between record row types when the data types can be converted to each other.
    pub fn convert<A: Into<D>>(
        RecordRow {
            data,
            canonical,
            modified,
        }: RecordRow<A>,
    ) -> Self {
        Self {
            data: data.into(),
            canonical,
            modified,
        }
    }
}

impl From<RawEntryData> for ArbitraryData {
    fn from(data: RawEntryData) -> Self {
        Self::Entry(data)
    }
}

impl From<Option<RemoteId>> for ArbitraryData {
    fn from(data: Option<RemoteId>) -> Self {
        Self::Deleted(data)
    }
}

impl From<()> for ArbitraryData {
    fn from((): ()) -> Self {
        Self::Void
    }
}

// Use a macro since the `From` conversion conflicts with the blanket implementation.
macro_rules! impl_row_from {
    ($($name:ty),*) => {
        $(
            impl From<RecordRow<$name>> for RecordRow<ArbitraryData> {
                fn from(row: RecordRow<$name>) -> Self {
                    Self::convert(row)
                }
            }
        )*
    };
}

impl_row_from!(RawEntryData, Option<RemoteId>, ());

impl<'conn> State<'conn, IsEntry> {
    /// Insert new data, preserving the old row as the parent row.
    pub fn modify(self, data: &RawEntryData) -> Result<Self, rusqlite::Error> {
        let new_key = self.replace_impl(data)?;
        Ok(Self::init(self.tx, IsEntry(new_key)))
    }

    /// Replace this row with a deletion marker, preserving the old row as the parent row.
    pub fn soft_delete(
        self,
        replacement: &Option<RemoteId>,
        update_aliases: bool,
    ) -> Result<State<'conn, IsDeleted>, rusqlite::Error> {
        let new_key = self.replace_impl(replacement)?;
        if update_aliases {
            match replacement {
                Some(canonical) => {
                    self.prepare(
                        "UPDATE Identifiers SET record_key = (SELECT record_key FROM Identifiers WHERE name = ?1) WHERE instr(name, ':') = 0 AND record_key = ?2",
                    )?
                    .execute((canonical.name(), new_key))?;
                }
                None => {
                    self.prepare(
                        "DELETE FROM Identifiers WHERE instr(name, ':') = 0 AND record_key = ?1",
                    )?
                    .execute([new_key])?;
                }
            }
        }
        Ok(State {
            tx: self.tx,
            id: IsDeleted(new_key),
        })
    }

    /// Add a new alias for this row.
    ///
    /// The return value is `false` if the alias already exists, and otherwise `true`.
    #[inline]
    pub fn add_alias(&self, alias: &Alias) -> Result<bool, rusqlite::Error> {
        self.add_refs_impl(std::iter::once(alias), IdentifierInsertMode::FailIfExists)
    }

    /// Update an existing alias to point to this row.
    ///
    /// The return value is `false` if the alias already exists, and otherwise `true`.
    #[inline]
    pub fn update_alias(&self, alias: &Alias) -> Result<bool, rusqlite::Error> {
        let rows_changed = self
            .prepare("UPDATE Identifiers SET record_key = ?1 WHERE name = ?2")?
            .execute((self.row_id(), alias.name()))?;
        Ok(rows_changed == 1)
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
                    Ok(Some(self.canonical()?))
                }
            }
            None => {
                self.prepare("INSERT INTO Identifiers (name, record_key) values (?1, ?2)")?
                    .execute((alias.name(), self.row_id()))?;
                Ok(None)
            }
        }
    }
}

impl<'conn, I: NotEntry> State<'conn, I> {
    /// Insert data for the void row, creating a new child row.
    pub fn reinsert(self, data: &RawEntryData) -> rusqlite::Result<State<'conn, IsEntry>> {
        let new_key = self.replace_impl(data)?;
        Ok(State::init(self.tx, IsEntry(new_key)))
    }
}

impl<'conn> State<'conn, IsArbitrary> {
    /// Disambiguate the arbitrary state, returning the data as well as the resulting type.
    pub fn disambiguate(self) -> Result<DisambiguatedRecordRow<'conn>, rusqlite::Error> {
        let RecordRow {
            data,
            modified,
            canonical,
        } = self.get_data()?;

        let row_id = self.row_id();

        Ok(match data {
            ArbitraryData::Entry(data) => DisambiguatedRecordRow::Entry(
                RecordRow {
                    data,
                    modified,
                    canonical,
                },
                State::init(self.tx, IsEntry(row_id)),
            ),
            ArbitraryData::Deleted(data) => DisambiguatedRecordRow::Deleted(
                RecordRow {
                    data,
                    modified,
                    canonical,
                },
                State::init(self.tx, IsDeleted(row_id)),
            ),
            ArbitraryData::Void => DisambiguatedRecordRow::Void(
                RecordRow {
                    data: (),
                    modified,
                    canonical,
                },
                State::init(self.tx, IsVoid(row_id)),
            ),
        })
    }
}

/// The type of identifier insertion to perform.
pub enum IdentifierInsertMode {
    /// Overwrite the existing identifier, if any.
    Overwrite,
    /// Fail if there is an existing identifier.
    FailIfExists,
    /// Ignore if there is an existing identifier.
    IgnoreIfExists,
}
