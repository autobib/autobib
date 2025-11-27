use chrono::{DateTime, Local};
use rusqlite::OptionalExtension;

use crate::{
    Alias, RawEntryData, RemoteId,
    db::{CitationKey, Constraint, flatten_constraint_violation, get_row_id},
    logger::debug,
};

use super::{Missing, State, Transaction, version::RevisionId};

/// Any type which represents a `key` corresponding to a row in the 'Records' table.
pub trait InRecordsTable {
    /// The data associated with the row.
    type Data: AsRecordRowData + FromBytesAndVariant;

    /// Convert to a row id.
    fn row_id(&self) -> i64;
}

/// Any type which represents a `key` corresponding to a row in the 'Records' table which is not a
/// 'void' row.
pub trait NotVoid: InRecordsTable {}

/// Any type which represents a `key` corresponding to a row in the 'Records' table which is not a
/// 'entry' row.
pub trait NotEntry: InRecordsTable {}

pub trait FromBytesAndVariant: Sized {
    fn from_bytes_and_variant(bytes: Vec<u8>, variant: i64) -> Self;
}

/// The data for a row in the 'Records' table, not including information about the parents or
/// children.
#[derive(Debug)]
pub struct RecordRow<D> {
    /// The associated data.
    pub data: D,
    /// The canonical identifier.
    pub canonical: RemoteId,
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
    pub(in crate::db) fn from_row_unchecked(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let data = D::from_bytes_and_variant(row.get("data")?, row.get("variant")?);
        let canonical = RemoteId::from_string_unchecked(row.get("record_id")?);
        let modified = row.get("modified")?;

        Ok(Self {
            data,
            modified,
            canonical,
        })
    }

    /// Load from a row id, which the caller promises is a valid row ID in the 'Records' table and
    /// moreover has type `D`.
    pub(super) fn load_unchecked(tx: &Transaction<'_>, row_id: i64) -> rusqlite::Result<Self> {
        tx.prepare_cached("SELECT record_id, modified, data, variant FROM Records WHERE key = ?1")?
            .query_row((row_id,), Self::from_row_unchecked)
    }
}

/// The data for a row in the 'Records' table, also including information about the parents and
/// children.
pub struct CompleteRecordRow<D> {
    pub row: RecordRow<D>,
    pub(super) parent: Option<i64>,
    pub(super) children: Vec<u8>,
}

impl<D: FromBytesAndVariant> CompleteRecordRow<D> {
    pub(super) fn load_unchecked(tx: &Transaction<'_>, row_id: i64) -> rusqlite::Result<Self> {
        tx.prepare_cached("SELECT record_id, modified, data, variant, parent_key, children FROM Records WHERE key = ?1")?
            .query_row((row_id,), |row| {
                let parent = row.get("parent_key")?;
                let children = row.get("children")?;
                let row = RecordRow::from_row_unchecked(row)?;

                Ok(Self {
                    row,
                    parent,
                    children,
                })
            })
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
pub struct ArbitraryKey(pub(super) i64);

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

impl_record_key!(ArbitraryKey, ArbitraryData);

/// The `key` of a row in the 'Records' table which is either an `entry` or `deleted`.
#[derive(Debug)]
pub struct EntryOrDeletedKey(pub(super) i64);

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

impl NotVoid for EntryOrDeletedKey {}

impl_record_key!(EntryOrDeletedKey, EntryOrDeletedData);

/// The `key` of a regular entry in the 'Records' table.
#[derive(Debug)]
pub struct EntryRecordKey(pub(super) i64);

impl FromBytesAndVariant for RawEntryData {
    fn from_bytes_and_variant(bytes: Vec<u8>, variant: i64) -> Self {
        assert!(
            variant == 0,
            "Unexpected 'Records' table row variant: expected entry data."
        );
        Self::from_byte_repr_unchecked(bytes)
    }
}

impl NotVoid for EntryRecordKey {}

impl_record_key!(EntryRecordKey, RawEntryData);

/// The `key` of a soft-deleted row in the 'Records' table.
#[derive(Debug)]
pub struct DeletedRecordKey(i64);

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

impl NotVoid for DeletedRecordKey {}
impl NotEntry for DeletedRecordKey {}

impl_record_key!(DeletedRecordKey, Option<RemoteId>);

/// The `key` of the 'void' root node in the 'Records' table, which is typically not stored in the
/// database at all (in
/// order to save database space) but
/// is created when required to undo into the deleted state which precedes all record of the data.
#[derive(Debug)]
pub struct VoidRecordKey(pub(super) i64);

impl FromBytesAndVariant for () {
    fn from_bytes_and_variant(_: Vec<u8>, variant: i64) -> Self {
        assert!(
            variant == 2,
            "Unexpected 'Records' table row variant: expected void marker"
        );
    }
}

impl NotEntry for VoidRecordKey {}

impl_record_key!(VoidRecordKey, ());

/// A row in the 'Records' table, disambiguated based on what type of row it is.
pub enum DisambiguatedRecordRow<'conn> {
    Entry(RecordRow<RawEntryData>, State<'conn, EntryRecordKey>),
    Deleted(RecordRow<Option<RemoteId>>, State<'conn, DeletedRecordKey>),
    Void(RecordRow<()>, State<'conn, VoidRecordKey>),
}

impl<'conn> DisambiguatedRecordRow<'conn> {
    pub fn forget(self) -> (RecordRow<ArbitraryData>, State<'conn, ArbitraryKey>) {
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
    pub(super) fn row_id(&self) -> i64 {
        self.id.row_id()
    }

    /// Perform unchecked conversion to a new row id, updating the rows in the CitationKeys table.
    fn transmute<N: FromRowId>(self, new_row_id: i64) -> rusqlite::Result<State<'conn, N>> {
        self.update_citation_keys(new_row_id)?;
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

    /// Get last modified time.
    #[inline]
    pub fn last_modified(&self) -> Result<DateTime<Local>, rusqlite::Error> {
        debug!("Getting last modified time for row '{}'.", self.row_id());
        get_last_modified(&self.tx, self.row_id())
    }

    /// Obtain the complete data for this row, which also contains information about parents and
    /// children.
    pub fn get_complete_data(&self) -> rusqlite::Result<CompleteRecordRow<I::Data>> {
        debug!(
            "Retrieving 'Records' data associated with row '{}'",
            self.row_id()
        );
        CompleteRecordRow::load_unchecked(&self.tx, self.row_id())
    }

    /// Forget the specific type of row that this is.
    pub fn forget(self) -> State<'conn, ArbitraryKey> {
        let row_id = self.row_id();
        State::init(self.tx, ArbitraryKey(row_id))
    }

    /// Update the active row to a specific revision.
    ///
    /// If the row-id does not correspond to a row in the 'Records' table with a canonical id which is the same
    /// as the canonical id of this row, this returns an error.
    pub fn set_active(
        self,
        RevisionId(row_id): RevisionId,
    ) -> Result<RecordRowMoveResult<'conn, ArbitraryKey, I, SetActiveError>, rusqlite::Error> {
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

    pub fn rewind(self, before: DateTime<Local>) -> rusqlite::Result<State<'conn, ArbitraryKey>> {
        // FIXME: avoid this?
        let canonical = self.canonical()?;
        let new_row_id: i64 = self.prepare("SELECT key FROM Records WHERE record_id = ?1 AND modified <= ?2 ORDER BY modified DESC LIMIT 1")?
            .query_row((canonical.name(), &before), |row| row.get(0))?;
        self.transmute(new_row_id)
    }

    /// Update the citation keys table by setting any rows which reference the current row to
    /// reference a new row id instead.
    fn update_citation_keys(&self, new_key: i64) -> Result<usize, rusqlite::Error> {
        self.prepare("UPDATE CitationKeys SET record_key = ?1 WHERE record_key = ?2")?
            .execute((new_key, self.row_id()))
    }

    /// Hard delete the row. This deletes every entry in the 'Records' with the same canonical
    /// identifier as the current row.
    pub fn hard_delete(self) -> Result<State<'conn, Missing>, rusqlite::Error> {
        self.prepare(
            "DELETE FROM Records WHERE record_id = (SELECT record_id FROM Records WHERE key = ?1);",
        )?
        .execute((self.row_id(),))?;

        Ok(State::init(self.tx, Missing))
    }

    /// Get every key in the `CitationKeys` table which references this row.
    pub fn referencing_keys(&self) -> Result<Vec<String>, rusqlite::Error> {
        self.referencing_keys_impl(Some)
    }

    /// Get every remote id in the `CitationKeys` table which references this row.
    pub fn referencing_remote_ids(&self) -> Result<Vec<RemoteId>, rusqlite::Error> {
        self.referencing_keys_impl(RemoteId::from_alias_or_remote_id_unchecked)
    }

    /// Get a transformed version of every key in the `CitationKeys` table which references
    /// the current row for which the provided `filter_map` does not return `None`.
    fn referencing_keys_impl<T, F: FnMut(String) -> Option<T>>(
        &self,
        mut filter_map: F,
    ) -> Result<Vec<T>, rusqlite::Error> {
        debug!("Getting referencing keys for '{}'.", self.row_id());
        let mut selector = self.prepare("SELECT name FROM CitationKeys WHERE record_key = ?1")?;
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
                CitationKeyInsertMode::Overwrite => {
                    "INSERT OR REPLACE INTO CitationKeys (name, record_key) values (?1, ?2)"
                }
                CitationKeyInsertMode::IgnoreIfExists => {
                    "INSERT OR IGNORE INTO CitationKeys (name, record_key) values (?1, ?2)"
                }
                CitationKeyInsertMode::FailIfExists => {
                    "INSERT INTO CitationKeys (name, record_key) values (?1, ?2)"
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

    /// Insert a new row with data, adding the previous row as the parent and appending this row as
    /// the child of the new row.
    fn replace_impl<R: AsRecordRowData>(&self, data: &R) -> Result<i64, rusqlite::Error> {
        // read the current value of 'record_id' and 'children'
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

        // update the `children` field with the existing records
        let mut new_children = existing.children;
        new_children.extend(new_key.to_le_bytes());
        self.prepare("UPDATE Records SET children = ?1 WHERE key = ?2")?
            .execute((new_children, self.row_id()))?;

        self.update_citation_keys(new_key)?;

        Ok(new_key)
    }

    fn redo_unchecked(
        self,
        index: Option<isize>,
    ) -> Result<RecordRowMoveResult<'conn, ArbitraryKey, I, RedoError>, rusqlite::Error> {
        let context = self.get_complete_data()?;
        match index {
            Some(idx) => {
                if idx >= 0 {
                    RecordRowMoveResult::from_rowid(
                        self,
                        context
                            .child_keys()
                            .nth(idx.abs_diff(0))
                            .ok_or_else(|| RedoError::OutOfBounds(context.child_count())),
                    )
                } else {
                    RecordRowMoveResult::from_rowid(
                        self,
                        context
                            .child_keys()
                            .nth_back(idx.abs_diff(-1))
                            .ok_or_else(|| RedoError::OutOfBounds(context.child_count())),
                    )
                }
            }
            None => RecordRowMoveResult::from_rowid(self, context.unique_child()),
        }
    }
}

pub enum UndoError {
    ParentExists,
    ParentDeleted,
    ParentVoidExists,
    ParentVoidMissing,
}

impl<'conn, I: NotVoid> State<'conn, I> {
    /// Update the active row to be the parent of this row, if it exists and is an entry.
    pub fn undo(
        self,
    ) -> rusqlite::Result<RecordRowMoveResult<'conn, EntryRecordKey, I, UndoError>> {
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
    ) -> rusqlite::Result<RecordRowMoveResult<'conn, DeletedRecordKey, I, UndoError>> {
        let row_id_or_err = match self.current()?.parent()? {
            Some(parent) => match parent.row.data {
                ArbitraryData::Entry(_) => Err(UndoError::ParentExists),
                ArbitraryData::Deleted(_) => Ok(parent.row_id),
                ArbitraryData::Void => Err(UndoError::ParentVoidExists),
            },
            None => Err(UndoError::ParentVoidMissing),
        };

        RecordRowMoveResult::from_rowid(self, row_id_or_err)
    }

    /// Void this record row.
    pub fn void(self) -> rusqlite::Result<State<'conn, VoidRecordKey>> {
        let root = self.current()?.root(true)?;
        let root_row_id = root.row_id;

        let new_row_id = match root.row.data {
            ArbitraryData::Deleted(_) | ArbitraryData::Entry(_) => {
                // create the void root
                let new_row_id: i64 = root.tx.prepare("INSERT INTO Records (record_id, data, modified, variant, children) VALUES (?1, ?2, ?3, ?4, ?5) RETURNING key")?
            .query_row((root.row.canonical.name(), ().data_blob(), DateTime::<Local>::MIN_UTC, ().variant(), root.row_id.to_le_bytes()), |row| row.get(0))?;

                // update the non-void root to reference the parent
                root.tx
                    .prepare("UPDATE Records SET parent_key = ?1 WHERE key = ?2")?
                    .execute((Some(new_row_id), root.row_id))?;

                new_row_id
            }
            ArbitraryData::Void => root_row_id,
        };
        self.update_citation_keys(new_row_id)?;
        Ok(State::init(self.tx, VoidRecordKey(new_row_id)))
    }
}

impl<'conn, I: NotEntry> State<'conn, I> {
    pub fn redo_deletion(
        self,
        index: Option<isize>,
    ) -> Result<RecordRowMoveResult<'conn, ArbitraryKey, I, RedoError>, rusqlite::Error> {
        self.redo_unchecked(index)
    }
}

pub enum RedoError {
    OutOfBounds(usize),
    ChildNotUnique(usize),
}

impl<'conn> State<'conn, EntryRecordKey> {
    /// Update the active row to be a child of this row.
    ///
    /// If `index` is none and there is a unique child, this method will succeed. Otherwise,
    /// attempt to set to the `nth` child, ordered from first to last, where negative indices count
    /// backwards.
    ///
    /// The returned index on error is the number of children.
    pub fn redo(
        self,
        index: Option<isize>,
    ) -> Result<RecordRowMoveResult<'conn, ArbitraryKey, EntryRecordKey, RedoError>, rusqlite::Error>
    {
        self.redo_unchecked(index)
    }
}

impl<D> RecordRow<D> {
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

impl From<RecordRow<RawEntryData>> for RecordRow<ArbitraryData> {
    fn from(row: RecordRow<RawEntryData>) -> Self {
        Self::convert(row)
    }
}

impl From<RecordRow<Option<RemoteId>>> for RecordRow<ArbitraryData> {
    fn from(row: RecordRow<Option<RemoteId>>) -> Self {
        Self::convert(row)
    }
}

impl From<RecordRow<()>> for RecordRow<ArbitraryData> {
    fn from(row: RecordRow<()>) -> Self {
        Self::convert(row)
    }
}

impl<D> CompleteRecordRow<D> {
    /// Returns an iterator over all child keys in order of creation.
    pub(super) fn child_keys(&self) -> impl DoubleEndedIterator<Item = i64> + ExactSizeIterator {
        self.children
            .as_chunks()
            .0
            .iter()
            .map(|chunk| i64::from_le_bytes(*chunk))
    }

    pub(super) fn child_count(&self) -> usize {
        self.children.len() / 8
    }

    /// Return the unique child if there is exactly one child, and the number of children if not.
    fn unique_child(&self) -> Result<i64, RedoError> {
        let count = self.child_count();
        if count == 1 {
            Ok(self.child_keys().next().unwrap())
        } else {
            Err(RedoError::ChildNotUnique(count))
        }
    }
}

impl<'conn> State<'conn, EntryRecordKey> {
    /// Insert new data, preserving the old row as the parent row.
    pub fn modify(self, data: &RawEntryData) -> Result<Self, rusqlite::Error> {
        let new_key = self.replace_impl(data)?;
        Ok(Self::init(self.tx, EntryRecordKey(new_key)))
    }

    /// Replace this row with a deletion marker, preserving the old row as the parent row.
    pub fn soft_delete(
        self,
        replacement: &Option<RemoteId>,
        update_aliases: bool,
    ) -> Result<State<'conn, DeletedRecordKey>, rusqlite::Error> {
        let new_key = self.replace_impl(replacement)?;
        if update_aliases {
            match replacement {
                Some(canonical) => {
                    self.prepare(
                        "UPDATE CitationKeys SET record_key = (SELECT record_key FROM CitationKeys WHERE name = ?1) WHERE instr(name, ':') = 0 AND record_key = ?2",
                    )?
                    .execute((canonical.name(), new_key))?;
                }
                None => {
                    self.prepare(
                        "DELETE FROM CitationKeys WHERE instr(name, ':') = 0 AND record_key = ?1",
                    )?
                    .execute([new_key])?;
                }
            }
        }
        Ok(State {
            tx: self.tx,
            id: DeletedRecordKey(new_key),
        })
    }

    /// Add a new alias for this row.
    ///
    /// The return value is `false` if the alias already exists, and otherwise `true`.
    #[inline]
    pub fn add_alias(&self, alias: &Alias) -> Result<bool, rusqlite::Error> {
        self.add_refs_impl(std::iter::once(alias), CitationKeyInsertMode::FailIfExists)
    }

    /// Update an existing alias to point to this row.
    ///
    /// The return value is `false` if the alias already exists, and otherwise `true`.
    #[inline]
    pub fn update_alias(&self, alias: &Alias) -> Result<bool, rusqlite::Error> {
        // self.add_refs_impl(std::iter::once(alias), CitationKeyInsertMode::Overwrite)
        let rows_changed = self
            .prepare("UPDATE CitationKeys SET record_key = ?1 WHERE name = ?2")?
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
                self.prepare("INSERT INTO CitationKeys (name, record_key) values (?1, ?2)")?
                    .execute((alias.name(), self.row_id()))?;
                Ok(None)
            }
        }
    }
}

impl<'conn, I: NotEntry> State<'conn, I> {
    /// Insert data for the void row
    pub fn reinsert(self, data: &RawEntryData) -> rusqlite::Result<State<'conn, EntryRecordKey>> {
        let new_key = self.replace_impl(data)?;
        Ok(State::init(self.tx, EntryRecordKey(new_key)))
    }
}

impl<'conn> State<'conn, ArbitraryKey> {
    pub fn determine(self) -> Result<DisambiguatedRecordRow<'conn>, rusqlite::Error> {
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
                State::init(self.tx, EntryRecordKey(row_id)),
            ),
            ArbitraryData::Deleted(data) => DisambiguatedRecordRow::Deleted(
                RecordRow {
                    data,
                    modified,
                    canonical,
                },
                State::init(self.tx, DeletedRecordKey(row_id)),
            ),
            ArbitraryData::Void => DisambiguatedRecordRow::Void(
                RecordRow {
                    data: (),
                    modified,
                    canonical,
                },
                State::init(self.tx, VoidRecordKey(row_id)),
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
