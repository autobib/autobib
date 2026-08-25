use std::{cmp::Reverse, fmt, io};

use autobib_entry::{Archive, data::EntryDataSerializer, v1::ArchivedEntryData};
use chrono::{DateTime, Local};
use rusqlite::OptionalExtension;
use serde::{Serialize, ser::SerializeStruct};

use crate::{
    Alias, Identifier,
    db::{
        AsKey, Constraint, flatten_constraint_violation, get_row_id,
        select::{AccessRowUnchecked, SelectOneUnchecked, stmt},
    },
    logger::{debug, info},
};

use super::{IsMissing, State, Tx, Updated, version::RevisionId};

/// Any state which represents a row in the 'Records' table.
pub trait InRecordsTable {
    /// The data associated with the row.
    type Data: AsRecordData + for<'a> AccessRowUnchecked<'a>;

    /// Convert to a row id.
    fn row_id(&self) -> i64;
}

/// Any state which represents a row in the 'Records' table which is not void.
pub trait NotVoid: InRecordsTable {}

/// Any state which represents a row in the 'Records' table which is not an entry.
pub trait NotEntry: InRecordsTable {}

/// The data for a row in the 'Records' table, not including information about the parents.
#[derive(Debug)]
pub struct Record<D = Box<ArchivedEntryData>, S = String> {
    /// The associated data.
    pub data: D,
    /// The canonical identifier.
    pub canonical: Identifier<S>,
    /// When the record was modified.
    pub modified: DateTime<Local>,
}

impl<D: AsRecordData, S: AsRef<str>> Record<D, S> {
    pub fn write_json_io<W: io::Write>(&self, writer: W) -> Result<(), io::Error> {
        Ok(serde_json::to_writer(writer, &self)?)
    }

    pub fn write_json_fmt<W: fmt::Write>(&self, writer: W) -> Result<(), fmt::Error> {
        // an adapter to use io writing methods with fmt
        struct FormatterAdapter<W: fmt::Write>(W);

        impl<W: fmt::Write> io::Write for FormatterAdapter<W> {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                let s = unsafe {
                    // serde_json does not emit invalid UTF-8
                    std::str::from_utf8_unchecked(buf)
                };

                match self.0.write_str(s) {
                    Ok(()) => Ok(s.len()),
                    Err(fmt::Error) => Err(io::ErrorKind::Other.into()),
                }
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let adapter = FormatterAdapter(writer);
        match serde_json::to_writer(adapter, &self) {
            Ok(()) => Ok(()),
            Err(err) => {
                if err.is_io() {
                    Err(fmt::Error)
                } else {
                    panic!("JSON serialization should not fail")
                }
            }
        }
    }
}

impl<D: AsRecordData, T: AsRef<str>> Serialize for Record<D, T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let field_count = if self.data.serializable() { 3 } else { 2 };

        let mut state = serializer.serialize_struct("Record", field_count)?;
        self.data.serialize_in(&mut state)?;
        state.serialize_field("canonical", self.canonical.as_key())?;
        state.serialize_field("modified", &self.modified)?;
        state.end()
    }
}

/// An inner type along with its revision id.
#[derive(Debug)]
pub struct WithRev<I> {
    pub inner: I,
    pub rev: RevisionId,
}

/// One of the three states that an entry can be in.
pub enum Variant {
    Entry,
    Deleted,
    Void,
}

/// The data for a row in the 'Records' table, also including information about the parents.
#[derive(Debug)]
pub struct HistRecord<D = ArbitraryData, S = String> {
    pub record: Record<D, S>,
    pub(in crate::db) parent: Option<i64>,
}

trait FromRowId: InRecordsTable {
    /// Construct from a row id.
    fn from_row_id(row_id: i64) -> Self;
}

macro_rules! impl_from_row_id {
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
    Entry(Box<ArchivedEntryData>),
    /// Deleted data.
    Deleted(Option<Identifier>),
    /// Void data.
    Void,
}

impl ArbitraryData {
    /// Get a reference to the data in this struct.
    pub fn as_deref(&self) -> ArbitraryDataRef<'_> {
        match self {
            Self::Entry(raw_entry_data) => ArbitraryDataRef::Entry(raw_entry_data.as_ref()),
            Self::Deleted(replacement) => {
                ArbitraryDataRef::Deleted(replacement.as_ref().map(Identifier::as_deref))
            }
            Self::Void => ArbitraryDataRef::Void,
        }
    }
}

impl_from_row_id!(IsArbitrary, ArbitraryData);

/// Equivalent to an [`ArbitraryData`], but borrows all of its data.
#[derive(Debug)]
pub enum ArbitraryDataRef<'r> {
    /// Entry data.
    Entry(&'r ArchivedEntryData),
    /// Deleted data.
    Deleted(Option<Identifier<&'r str>>),
    /// Void data.
    Void,
}

impl<'r> ArbitraryDataRef<'r> {
    pub fn as_owned(&self) -> ArbitraryData {
        match self {
            Self::Entry(archived_entry_data) => {
                ArbitraryData::Entry((*archived_entry_data).to_owned())
            }
            Self::Deleted(identifier) => {
                ArbitraryData::Deleted(identifier.as_ref().map(Identifier::as_owned))
            }
            Self::Void => ArbitraryData::Void,
        }
    }
}

/// The `key` of a row in the 'Records' table which is either an `entry` or `deleted`.
#[derive(Debug)]
pub struct IsEntryOrDeleted(pub(super) i64);

/// The row data associated with a row in the `Records` table. The precise value depends on the
/// `variant` column.
#[derive(Debug)]
pub enum NotVoidData {
    /// Entry data.
    Entry(Box<ArchivedEntryData>),
    /// Deleted data.
    Deleted(Option<Identifier>),
}

impl NotVoid for IsEntryOrDeleted {}

impl_from_row_id!(IsEntryOrDeleted, NotVoidData);

/// An entry in the 'Records' table.
#[derive(Debug)]
pub struct IsEntry(pub(super) i64);

impl NotVoid for IsEntry {}

impl_from_row_id!(IsEntry, Box<ArchivedEntryData>);

/// A deletion marker in the 'Records' table.
#[derive(Debug)]
pub struct IsDeleted(i64);

impl NotVoid for IsDeleted {}
impl NotEntry for IsDeleted {}

impl_from_row_id!(IsDeleted, Option<Identifier>);

/// The 'void' root node in the 'Records' table.
///
/// In order to save database state, this type is typically not stored at all,
/// is created when required to undo into the deleted state which precedes all record of the data.
#[derive(Debug)]
pub struct IsVoid(pub(super) i64);

impl NotEntry for IsVoid {}

impl_from_row_id!(IsVoid, ());

/// A row in the 'Records' table, disambiguated based on what type of row it is.
pub enum DisambiguatedRecordState<'conn> {
    Entry(Record, State<'conn, IsEntry>),
    Deleted(Record<Option<Identifier>>, State<'conn, IsDeleted>),
    Void(Record<()>, State<'conn, IsVoid>),
}

impl<'conn> DisambiguatedRecordState<'conn> {
    pub fn forget(self) -> (Record<ArbitraryData>, State<'conn, IsArbitrary>) {
        match self {
            Self::Entry(data, state) => (data.into(), state.forget()),
            Self::Deleted(data, state) => (data.into(), state.forget()),
            Self::Void(data, state) => (data.into(), state.forget()),
        }
    }
}

/// Types which can be written as the 'data' and 'variant' column in the 'Records' table.
pub trait AsRecordData {
    fn data_blob(&self) -> &[u8];

    fn variant(&self) -> i64;

    fn serializable(&self) -> bool;

    /// Serialize the data if serializable; otherwise, do nothing.
    fn serialize_in<S: SerializeStruct>(&self, ser_struct: &mut S) -> Result<(), S::Error>;
}

impl<T: AsRecordData + ?Sized> AsRecordData for &T {
    fn data_blob(&self) -> &[u8] {
        (*self).data_blob()
    }

    fn variant(&self) -> i64 {
        (*self).variant()
    }

    fn serializable(&self) -> bool {
        (*self).serializable()
    }

    fn serialize_in<S: SerializeStruct>(&self, ser_struct: &mut S) -> Result<(), S::Error> {
        (*self).serialize_in(ser_struct)
    }
}

impl AsRecordData for ArchivedEntryData {
    fn data_blob(&self) -> &[u8] {
        self.as_bytes()
    }

    fn variant(&self) -> i64 {
        0
    }

    fn serializable(&self) -> bool {
        true
    }

    fn serialize_in<S: SerializeStruct>(&self, ser_struct: &mut S) -> Result<(), S::Error> {
        ser_struct.serialize_field("data", &EntryDataSerializer::new(self))
    }
}

impl AsRecordData for Box<ArchivedEntryData> {
    #[inline]
    fn data_blob(&self) -> &[u8] {
        self.as_ref().data_blob()
    }

    #[inline]
    fn variant(&self) -> i64 {
        self.as_ref().variant()
    }

    #[inline]
    fn serializable(&self) -> bool {
        self.as_ref().serializable()
    }

    #[inline]
    fn serialize_in<S: SerializeStruct>(&self, ser_struct: &mut S) -> Result<(), S::Error> {
        self.as_ref().serialize_in(ser_struct)
    }
}

impl AsRecordData for Option<&Identifier> {
    fn data_blob(&self) -> &[u8] {
        self.map_or(b"", |r| r.as_key().as_bytes())
    }

    fn variant(&self) -> i64 {
        1
    }

    fn serializable(&self) -> bool {
        true
    }

    fn serialize_in<S: SerializeStruct>(&self, ser_struct: &mut S) -> Result<(), S::Error> {
        ser_struct.serialize_field("replacement", &self.as_ref().map(|i| i.as_key()))
    }
}

impl AsRecordData for Option<Identifier> {
    #[inline]
    fn data_blob(&self) -> &[u8] {
        self.as_ref().map_or(b"", |r| r.as_key().as_bytes())
    }

    #[inline]
    fn variant(&self) -> i64 {
        self.as_ref().variant()
    }

    #[inline]
    fn serializable(&self) -> bool {
        self.as_ref().serializable()
    }

    #[inline]
    fn serialize_in<S: SerializeStruct>(&self, ser_struct: &mut S) -> Result<(), S::Error> {
        self.as_ref().serialize_in(ser_struct)
    }
}

impl AsRecordData for () {
    fn data_blob(&self) -> &[u8] {
        &[]
    }

    fn variant(&self) -> i64 {
        2
    }

    fn serializable(&self) -> bool {
        false
    }

    fn serialize_in<S: SerializeStruct>(&self, _: &mut S) -> Result<(), S::Error> {
        Ok(())
    }
}

impl AsRecordData for NotVoidData {
    fn data_blob(&self) -> &[u8] {
        match self {
            Self::Entry(raw_entry_data) => raw_entry_data.data_blob(),
            Self::Deleted(id) => id.data_blob(),
        }
    }

    fn variant(&self) -> i64 {
        match self {
            Self::Entry(raw_entry_data) => raw_entry_data.variant(),
            Self::Deleted(id) => id.variant(),
        }
    }

    fn serializable(&self) -> bool {
        match self {
            Self::Entry(raw_entry_data) => raw_entry_data.serializable(),
            Self::Deleted(id) => id.serializable(),
        }
    }

    fn serialize_in<S: SerializeStruct>(&self, ser_struct: &mut S) -> Result<(), S::Error> {
        match self {
            Self::Entry(raw_entry_data) => raw_entry_data.serialize_in(ser_struct),
            Self::Deleted(id) => id.serialize_in(ser_struct),
        }
    }
}

impl AsRecordData for ArbitraryData {
    fn data_blob(&self) -> &[u8] {
        match self {
            Self::Entry(raw_entry_data) => raw_entry_data.data_blob(),
            Self::Deleted(id) => id.data_blob(),
            Self::Void => ().data_blob(),
        }
    }

    fn variant(&self) -> i64 {
        match self {
            Self::Entry(raw_entry_data) => raw_entry_data.variant(),
            Self::Deleted(id) => id.variant(),
            Self::Void => ().variant(),
        }
    }

    fn serializable(&self) -> bool {
        match self {
            Self::Entry(raw_entry_data) => raw_entry_data.serializable(),
            Self::Deleted(id) => id.serializable(),
            Self::Void => ().serializable(),
        }
    }

    fn serialize_in<S: SerializeStruct>(&self, ser_struct: &mut S) -> Result<(), S::Error> {
        match self {
            Self::Entry(raw_entry_data) => raw_entry_data.serialize_in(ser_struct),
            Self::Deleted(id) => id.serialize_in(ser_struct),
            Self::Void => ().serialize_in(ser_struct),
        }
    }
}

/// Get the canonical identifier.
fn get_canonical(tx: &Tx, row_id: i64) -> rusqlite::Result<Identifier> {
    tx.prepare_cached("SELECT canonical FROM Records WHERE rev = ?1")?
        .query_row([row_id], |row| {
            row.get("canonical").map(Identifier::from_string_unchecked)
        })
}

/// Get the last modified time.
fn get_last_modified(tx: &Tx, row_id: i64) -> rusqlite::Result<DateTime<Local>> {
    tx.prepare_cached("SELECT modified FROM Records WHERE rev = ?1")?
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
    DifferentCanonical(Identifier),
}

impl<'conn, I: InRecordsTable> State<'conn, I> {
    pub(in crate::db) fn row_id(&self) -> i64 {
        self.id.row_id()
    }

    /// Hard delete the row. This deletes every entry in the 'Records' with the same canonical
    /// identifier as the current row.
    pub fn delete_hard(self) -> Result<State<'conn, IsMissing>, rusqlite::Error> {
        debug!(
            "Permanently deleting all revisions in the edit-tree associated with the revision '{}'",
            self.row_id()
        );
        self.prepare(
            "DELETE FROM Records WHERE canonical IN (SELECT canonical FROM Records WHERE rev = ?1);",
        )?
        .execute((self.row_id(),))?;

        Ok(State::init(self.tx, IsMissing))
    }

    /// Unchecked conversion with a new row id of any type, updating the rows in the Keys table.
    fn transmute<N: FromRowId>(self, new_row_id: i64) -> rusqlite::Result<State<'conn, N>> {
        self.update_identifier_lookup(new_row_id)?;
        Ok(State::init(self.tx, N::from_row_id(new_row_id)))
    }

    /// Obtain the data for this row.
    pub fn get_data(&self) -> rusqlite::Result<Record<I::Data>> {
        debug!(
            "Retrieving record data associated with revision '{}'",
            self.row_id()
        );
        use crate::db::select::SelectOneUnchecked;
        stmt::GetArbitraryRecord::select_one_unchecked_cast(&self.tx, self.row_id())
    }

    /// Get the canonical [`Identifier`].
    #[inline]
    pub fn canonical(&self) -> Result<Identifier, rusqlite::Error> {
        debug!("Getting canonical identifier for '{}'.", self.row_id());
        get_canonical(&self.tx, self.row_id())
    }

    /// Get the hexadecimal revision of the active row.
    pub fn rev(&self) -> RevisionId {
        RevisionId(self.row_id())
    }

    /// Get last modified time.
    #[inline]
    pub fn last_modified(&self) -> Result<DateTime<Local>, rusqlite::Error> {
        debug!(
            "Getting last modified time for revision '{}'.",
            self.row_id()
        );
        get_last_modified(&self.tx, self.row_id())
    }

    /// Obtain the complete data for this row.
    pub fn get_complete_data(&self) -> rusqlite::Result<HistRecord<I::Data, String>> {
        debug!(
            "Retrieving record data associated with revision '{}'",
            self.row_id()
        );
        stmt::GetHist::select_one_unchecked_cast(&self.tx, self.row_id())
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
            "Updating the active revision for '{}' to '{}'.",
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
            "Rewinding from revision '{}' to how it looked at {}",
            self.row_id(),
            before
        );
        let canonical = self.canonical()?;
        let new_id = create_rewind_target(&self.tx, canonical.as_key(), before)?;
        self.transmute(new_id)
    }

    /// Update the 'Keys' table by setting any rows which reference the current row to
    /// reference a new row id instead.
    fn update_identifier_lookup(&self, new_key: i64) -> Result<usize, rusqlite::Error> {
        self.prepare("UPDATE Keys SET record_rev = ?1 WHERE record_rev = ?2")?
            .execute((new_key, self.row_id()))
    }

    /// Get every key in the `Keys` table which references this row.
    pub fn referencing_keys(&self) -> Result<Vec<String>, rusqlite::Error> {
        self.referencing_keys_impl(|k| Some(k.to_owned()))
    }

    /// Apply a mutable closure to every key in the `Keys` table which references this row.
    pub fn map_referencing_keys(&self, mut f: impl FnMut(&str)) -> Result<(), rusqlite::Error> {
        debug!("Getting referencing keys for '{}'.", self.row_id());
        let mut selector = self.prepare("SELECT name FROM Keys WHERE record_rev = ?1")?;
        let mut rows = selector.query((self.row_id(),))?;
        while let Some(row) = rows.next()? {
            if let rusqlite::types::ValueRef::Text(bytes) = row.get_ref_unwrap(0) {
                f(std::str::from_utf8(bytes).unwrap());
            } else {
                panic!("Keys table has unexpected schema: column 'name' is not a TEXT!");
            }
        }
        Ok(())
    }

    /// Get every remote id in the `Keys` table which references this row.
    pub fn referencing_ids(&self) -> Result<Vec<Identifier>, rusqlite::Error> {
        self.referencing_keys_impl(|k| Identifier::from_key_unchecked(k.to_owned()))
    }

    /// Get a transformed version of every key in the `Keys` table which references
    /// the current row for which the provided `filter_map` does not return `None`.
    fn referencing_keys_impl<T, F: FnMut(&str) -> Option<T>>(
        &self,
        mut filter_map: F,
    ) -> Result<Vec<T>, rusqlite::Error> {
        let mut referencing = Vec::with_capacity(1);
        self.map_referencing_keys(|k| {
            if let Some(mapped) = filter_map(k) {
                referencing.push(mapped);
            }
        })?;
        Ok(referencing)
    }

    /// Insert [`Identifier`] references for this row.
    ///
    /// The return value is `false` if the insertion failed and `IdentifierInsertMode` is
    /// `FailIfExists`, and otherwise `true`.
    #[inline]
    pub fn add_refs<'a, R: Iterator<Item = &'a Identifier>>(
        &self,
        refs: R,
    ) -> Result<bool, rusqlite::Error> {
        self.add_refs_impl(refs, IdentifierInsertMode::Overwrite)
    }

    /// Insert [`Identifier`] references for this row.
    ///
    /// The return value is `false` if the insertion failed and `IdentifierInsertMode` is
    /// `FailIfExists`, and otherwise `true`.
    fn add_refs_impl<'a, K: AsKey + 'a, R: Iterator<Item = &'a K>>(
        &self,
        refs: R,
        mode: IdentifierInsertMode,
    ) -> Result<bool, rusqlite::Error> {
        debug!("Inserting references to revision '{}'", self.row_id());
        for id in refs {
            let stmt = match mode {
                IdentifierInsertMode::Overwrite => {
                    "INSERT OR REPLACE INTO Keys (name, record_rev) values (?1, ?2)"
                }
                IdentifierInsertMode::IgnoreIfExists => {
                    "INSERT OR IGNORE INTO Keys (name, record_rev) values (?1, ?2)"
                }
                IdentifierInsertMode::FailIfExists => {
                    "INSERT INTO Keys (name, record_rev) values (?1, ?2)"
                }
            };
            let mut key_writer = self.prepare(stmt)?;
            match flatten_constraint_violation(key_writer.execute((id.as_key(), self.row_id())))? {
                Constraint::Satisfied(_) => {}
                Constraint::Violated => return Ok(false),
            }
        }
        Ok(true)
    }

    /// Insert a new row with data, adding the previous row as the parent.
    fn replace_impl<R: AsRecordData + ?Sized>(
        &self,
        data: &R,
    ) -> Result<(i64, DateTime<Local>), rusqlite::Error> {
        let existing = self.get_complete_data()?;

        // insert a new row into Records containing:
        //
        // - the previous value of 'key'
        // - the new data
        // - the current timestamp
        // - the correct variant
        // - the key of the row being replaced, in parent_rev
        //
        // the remaining fields use their default values
        let dt = Local::now();
        let new_key: i64 = self.prepare("INSERT INTO Records (canonical, data, modified, variant, parent_rev) VALUES (?1, ?2, ?3, ?4, ?5) RETURNING rev")?
            .query_row((existing.record.canonical.as_key(), data.data_blob(), &dt, data.variant(), self.row_id()), |row| row.get(0))?;

        self.update_identifier_lookup(new_key)?;

        Ok((new_key, dt))
    }

    /// Perform a redo operation from an arbitrary state.
    fn redo_unchecked(
        self,
        idx: isize,
    ) -> Result<RecordRowMoveResult<'conn, IsArbitrary, I, RedoError>, rusqlite::Error> {
        let version = self.current()?;
        let mut children = Vec::new();
        version.map_children(
            |WithRev {
                 inner: data,
                 rev: row_id,
             }| {
                children.push((data.record.modified, row_id.0));
            },
        )?;

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
            Some(parent) => match parent.hist.record.data {
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
            Some(parent) => match parent.hist.record.data {
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

        let new_row_id = match root.hist.record.data {
            ArbitraryData::Deleted(_) | ArbitraryData::Entry(_) => {
                create_void_parent(root.tx, root.row_id, root.hist.record.canonical.as_key())?
            }
            ArbitraryData::Void => root_row_id,
        };
        self.update_identifier_lookup(new_row_id)?;
        Ok(State::init(self.tx, IsVoid(new_row_id)))
    }
}

/// Replace the row at `original` with the row at `target`.
///
/// The caller is required to guarantee that:
///
/// 1. `original` is the row-id of an active row in the 'Records' table with canonical
///    `original_canonical`.
/// 2. `target` is a valid row-id of another active row in the 'Records table', which does not have
///    canonical id `original_canonical`.
pub fn replace_hard_unchecked<'conn>(
    tx: Tx<'conn>,
    original: IsEntry,
    original_canonical: &Identifier,
    target: IsEntry,
) -> rusqlite::Result<Tx<'conn>> {
    tx.prepare("UPDATE Keys SET record_rev = ?1 WHERE record_rev = ?2")?
        .execute((target.0, original.0))?;

    tx.prepare("DELETE FROM Records WHERE canonical = ?1")?
        .execute([original_canonical.as_key()])?;

    Ok(tx)
}

/// Returns a row which can be the target to rewind to.
///
/// If a row exists which has modification time before `before`, this row is returned. Otherwise,
/// a new void root is created, and the row id is returned.
pub(in crate::db) fn create_rewind_target(
    tx: &Tx<'_>,
    canonical: &str,
    before: DateTime<Local>,
) -> rusqlite::Result<i64> {
    // first, try to find a candidate vertex to swap to
    let id_opt: Option<i64> = tx.prepare("SELECT rev FROM Records WHERE canonical = ?1 AND modified <= ?2 ORDER BY modified DESC LIMIT 1")?
            .query_row((canonical, before), |row| row.get(0)).optional()?;

    Ok(if let Some(id) = id_opt {
        id
    } else {
        // if no candidate exists, this means the modified time is > `before` on every entry in
        // canonical, so we find the root and add the void vertex before it
        let root_row_id: i64 = tx
            .prepare("SELECT rev FROM Records WHERE canonical = ?1 AND parent_rev IS NULL")?
            .query_row([canonical], |row| row.get(0))?;
        create_void_parent(tx, root_row_id, canonical)?
    })
}

/// Create a parent to this row which is a void record.
fn create_void_parent(tx: &Tx<'_>, root_row_id: i64, canonical: &str) -> rusqlite::Result<i64> {
    // create the void root
    let new_row_id: i64 = tx.prepare("INSERT INTO Records (canonical, data, modified, variant) VALUES (?1, ?2, ?3, ?4) RETURNING rev")?
            .query_row((canonical, ().data_blob(), DateTime::<Local>::MIN_UTC, ().variant()), |row| row.get(0))?;

    // update the non-void root to reference the parent
    tx.prepare("UPDATE Records SET parent_rev = ?1 WHERE rev = ?2")?
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
        candidate: &Identifier,
        update_aliases: bool,
    ) -> rusqlite::Result<RecordRowMoveResult<'conn, IsDeleted, IsEntry, bool>> {
        let replacement: Option<i64> = self
            .tx
            .prepare("SELECT record_rev FROM Keys WHERE name = ?1")?
            .query_row([candidate.as_key()], |row| row.get("record_rev"))
            .optional()?;

        match replacement {
            None => Ok(RecordRowMoveResult::Unchanged(self, false)),
            Some(row_id) => {
                if row_id == self.row_id() {
                    Ok(RecordRowMoveResult::Unchanged(self, true))
                } else {
                    let repl: String = self
                        .tx
                        .prepare("SELECT canonical FROM Records WHERE rev = ?1")?
                        .query_row([row_id], |row| row.get("canonical"))?;
                    let id = Identifier::from_string_unchecked(repl);
                    info!("Replacing record with new canonical id '{id}'");
                    let deleted = self.delete_soft(Some(&id), update_aliases)?.state;
                    Ok(RecordRowMoveResult::Updated(deleted))
                }
            }
        }
    }
}

impl<D> Record<D> {
    /// Convert between record row types when the data types can be converted to each other.
    pub fn convert<A: Into<D>>(
        Record {
            data,
            canonical,
            modified,
        }: Record<A>,
    ) -> Self {
        Self {
            data: data.into(),
            canonical,
            modified,
        }
    }
}

impl From<Box<ArchivedEntryData>> for ArbitraryData {
    fn from(data: Box<ArchivedEntryData>) -> Self {
        Self::Entry(data)
    }
}

impl From<Option<Identifier>> for ArbitraryData {
    fn from(data: Option<Identifier>) -> Self {
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
            impl From<Record<$name>> for Record<ArbitraryData> {
                fn from(row: Record<$name>) -> Self {
                    Self::convert(row)
                }
            }
        )*
    };
}

impl_row_from!(Box<ArchivedEntryData>, Option<Identifier>, ());

impl<'conn> State<'conn, IsEntry> {
    /// Insert new data, preserving the old row as the parent row.
    pub fn modify(
        self,
        data: &ArchivedEntryData,
    ) -> Result<Updated<'conn, IsEntry>, rusqlite::Error> {
        let (new_key, modified) = self.replace_impl(data)?;
        Ok(Self::init(self.tx, IsEntry(new_key)).with_timestamp(modified))
    }

    /// Create a new row which is a copy of the current row but with an updated modification time.
    pub fn touch(self) -> rusqlite::Result<Self> {
        self.touch_with_timestamp(&Local::now())
    }

    /// Create a new row which is a copy of the current row but with the provided modification
    /// time.
    pub fn touch_with_timestamp(self, dt: &DateTime<Local>) -> rusqlite::Result<Self> {
        // TODO: this is never used / tested
        let new_row_id: i64 = self
            .tx
            .prepare(
                "
INSERT INTO Records (canonical, data, modified, variant, parent_rev)
SELECT canonical, data, ?1, variant, rev
FROM Records
WHERE rev = ?2
RETURNING rev",
            )?
            .query_row((dt, self.row_id()), |row| row.get("rev"))?;
        self.transmute(new_row_id)
    }

    /// Replace this row with a deletion marker, preserving the old row as the parent row.
    pub fn delete_soft(
        self,
        replacement: Option<&Identifier>,
        update_aliases: bool,
    ) -> Result<Updated<'conn, IsDeleted>, rusqlite::Error> {
        let (new_key, modified) = self.replace_impl(&replacement)?;
        if update_aliases {
            match replacement {
                Some(canonical) => {
                    self.prepare(
                        "UPDATE Keys SET record_rev = (SELECT record_rev FROM Keys WHERE name = ?1) WHERE instr(name, ':') = 0 AND record_rev = ?2",
                    )?
                    .execute((canonical.as_key(), new_key))?;
                }
                None => {
                    self.prepare(
                        "DELETE FROM Keys WHERE instr(name, ':') = 0 AND record_rev = ?1",
                    )?
                    .execute([new_key])?;
                }
            }
        }
        Ok(State {
            tx: self.tx,
            id: IsDeleted(new_key),
        }
        .with_timestamp(modified))
    }

    /// Add a new alias for this row.
    ///
    /// This method returns `None` if the alias was newly cretaed, or `Some(canonical_identifier)`
    /// if an alias already exists, containing the canonical id of the existing alias.
    #[inline]
    pub fn add_alias(&mut self, alias: &Alias) -> Result<Option<Identifier>, rusqlite::Error> {
        if self.add_refs_impl(std::iter::once(alias), IdentifierInsertMode::FailIfExists)? {
            Ok(None)
        } else {
            let existing_row_id = get_row_id(&self.tx, alias)?
                .expect("Alias must exist after its insertion violated the unique constraint");
            get_canonical(&self.tx, existing_row_id).map(Some)
        }
    }

    /// Reassign an existing alias to point to this row.
    #[inline]
    pub fn reassign_alias(
        &mut self,
        alias: &Alias,
    ) -> Result<ReassignAliasResult, rusqlite::Error> {
        let rows_changed = self
            .prepare("UPDATE Keys SET record_rev = ?1 WHERE name = ?2")?
            .execute((self.row_id(), alias.as_key()))?;
        if rows_changed == 0 {
            Ok(ReassignAliasResult::Missing)
        } else {
            Ok(ReassignAliasResult::Reassigned)
        }
    }

    /// Ensure that the given alias exists for this row.
    ///
    /// If the alias already exists and points to a different row, the canonical id of the other row is returned.
    #[inline]
    pub fn ensure_alias(&mut self, alias: &Alias) -> Result<Option<Identifier>, rusqlite::Error> {
        debug!(
            "Ensuring alias '{alias}' refers to revision '{}'",
            self.row_id()
        );
        match get_row_id(&self.tx, alias)? {
            Some(existing_row_id) => {
                if existing_row_id == self.row_id() {
                    Ok(None)
                } else {
                    get_canonical(&self.tx, existing_row_id).map(Some)
                }
            }
            None => {
                self.prepare("INSERT INTO Keys (name, record_rev) values (?1, ?2)")?
                    .execute((alias.as_key(), self.row_id()))?;
                Ok(None)
            }
        }
    }
}

#[must_use]
pub enum ReassignAliasResult {
    /// The alias was successfuly reassigned to a new value.
    Reassigned,
    /// The alias was missing and could not be reassigned.
    Missing,
}

impl<'conn, I: NotEntry> State<'conn, I> {
    /// Insert data for the void row, creating a new child row.
    pub fn reinsert(self, data: &ArchivedEntryData) -> rusqlite::Result<Updated<'conn, IsEntry>> {
        let (new_key, modified) = self.replace_impl(data)?;
        Ok(State::init(self.tx, IsEntry(new_key)).with_timestamp(modified))
    }
}

impl<'conn> State<'conn, IsArbitrary> {
    /// Disambiguate the arbitrary state, returning the data as well as the resulting type.
    pub fn disambiguate(self) -> Result<DisambiguatedRecordState<'conn>, rusqlite::Error> {
        let Record {
            data,
            modified,
            canonical,
        } = self.get_data()?;

        let row_id = self.row_id();

        Ok(match data {
            ArbitraryData::Entry(data) => DisambiguatedRecordState::Entry(
                Record {
                    data,
                    modified,
                    canonical,
                },
                State::init(self.tx, IsEntry(row_id)),
            ),
            ArbitraryData::Deleted(data) => DisambiguatedRecordState::Deleted(
                Record {
                    data,
                    modified,
                    canonical,
                },
                State::init(self.tx, IsDeleted(row_id)),
            ),
            ArbitraryData::Void => DisambiguatedRecordState::Void(
                Record {
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
