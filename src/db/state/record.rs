use chrono::{DateTime, Local};
use rusqlite::OptionalExtension;

use crate::{
    Alias, RawEntryData, RemoteId,
    db::{CitationKey, Constraint, flatten_constraint_violation, get_row_id},
    logger::debug,
};

use super::{Missing, State, Transaction};

/// An unvalidated `key`, which may or may not correspond to a row in the 'Records' table.
///
/// Keys are represented as hexadecimal strings prefixed by the `#` character, and can be parsed in a
/// case-insensitive fashion.
///
/// In principle, keys could be negative, but in practice, SQLite will never insert negative values
/// for the key automatically (unless negative keys are already present), and we never manually input values for the `key` column.
pub struct Unvalidated(i64);

macro_rules! impl_key_display {
    ($v:ident) => {
        impl std::fmt::Display for $v {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "#{:x}", self.0)
            }
        }
    };
}

impl_key_display!(Unvalidated);
impl_key_display!(RecordKey);
impl_key_display!(DeletedRecordKey);
impl_key_display!(EntryRecordKey);

impl std::str::FromStr for Unvalidated {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        i64::from_str_radix(s, 16).map(Self)
    }
}

/// The `key` of a row in the 'Records' table, with unknown type.
#[derive(Debug)]
pub struct RecordKey(pub(super) i64);

/// The `key` of a soft-deleted row in the 'Records' table.
#[derive(Debug)]
pub struct DeletedRecordKey(i64);

/// The `key` of a regular entry in the 'Records' table.
#[derive(Debug)]
pub struct EntryRecordKey(pub(super) i64);

/// Any type which represents a `key` corresponding to a row in the 'Records' table.
pub trait AsRecordKey {
    fn row_id(&self) -> i64;
}

impl AsRecordKey for EntryRecordKey {
    fn row_id(&self) -> i64 {
        self.0
    }
}

impl AsRecordKey for DeletedRecordKey {
    fn row_id(&self) -> i64 {
        self.0
    }
}
impl AsRecordKey for RecordKey {
    fn row_id(&self) -> i64 {
        self.0
    }
}

/// Types which can be written as the 'data' and 'variant' column in the 'Records' table.
trait RecordsDataCol {
    fn data_blob(&self) -> &[u8];

    fn variant(&self) -> i64;
}

impl RecordsDataCol for RawEntryData {
    fn data_blob(&self) -> &[u8] {
        self.to_byte_repr()
    }

    fn variant(&self) -> i64 {
        0
    }
}

impl RecordsDataCol for Option<RemoteId> {
    fn data_blob(&self) -> &[u8] {
        self.as_ref().map_or(b"", |r| r.name().as_bytes())
    }

    fn variant(&self) -> i64 {
        1
    }
}

impl RecordsDataCol for EntryDataOrReplacement {
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

/// Types which can be converted from a row in the 'Records' table, assuming the row has the
/// correct state.
pub(super) trait RecordsLookup<I: AsRecordKey>: Sized {
    fn lookup_unchecked(tx: &Transaction, row_id: i64) -> rusqlite::Result<Self>;

    fn lookup<'conn>(state: &State<'conn, I>) -> Result<Self, rusqlite::Error> {
        Self::lookup_unchecked(&state.tx, state.row_id())
    }
}

impl<I: AsRecordKey> RecordsLookup<I> for DateTime<Local> {
    fn lookup_unchecked(tx: &Transaction, row_id: i64) -> Result<Self, rusqlite::Error> {
        tx.prepare("SELECT modified FROM Records WHERE key = ?1")?
            .query_row([row_id], |row| row.get("modified"))
    }
}

impl<I: AsRecordKey> RecordsLookup<I> for RemoteId {
    fn lookup_unchecked(tx: &Transaction, row_id: i64) -> rusqlite::Result<Self> {
        tx.prepare("SELECT record_id FROM Records WHERE key = ?1")?
            .query_row([row_id], |row| {
                row.get("record_id").map(Self::from_string_unchecked)
            })
    }
}

impl<I: AsRecordKey> RecordsLookup<I> for RecordRowData {
    fn lookup_unchecked(tx: &Transaction, row_id: i64) -> rusqlite::Result<Self> {
        tx.prepare_cached("SELECT record_id, modified, data, variant FROM Records WHERE key = ?1")?
            .query_row((row_id,), |row| {
                let data = EntryDataOrReplacement::from_bytes_and_variant(
                    row.get("data")?,
                    row.get("variant")?,
                );
                let canonical = RemoteId::from_string_unchecked(row.get("record_id")?);
                let modified = row.get("modified")?;

                Ok(Self {
                    data,
                    modified,
                    canonical,
                })
            })
    }
}

impl RecordsLookup<EntryRecordKey> for EntryRowData {
    fn lookup_unchecked(tx: &Transaction, row_id: i64) -> rusqlite::Result<Self> {
        tx.prepare_cached("SELECT record_id, modified, data, variant FROM Records WHERE key = ?1")?
            .query_row((row_id,), |row| row.try_into())
    }
}

impl<I: AsRecordKey> RecordsLookup<I> for RecordContext {
    fn lookup_unchecked(tx: &Transaction, row_id: i64) -> rusqlite::Result<Self> {
        tx.prepare_cached(
            "SELECT record_id, data, modified, variant, parent_key, children FROM Records WHERE key = ?1",
        )?
        .query_row((row_id,), |row| {
            let data = EntryDataOrReplacement::from_bytes_and_variant(
                row.get("data")?,
                row.get("variant")?,
            );
            let canonical = RemoteId::from_string_unchecked(row.get("record_id")?);
            let modified = row.get("modified")?;
            let parent = row.get("parent_key")?;
            let children = row.get("children")?;

            Ok(Self {
                canonical,
                data,
                modified,
                parent,
                children,
            })
        })
    }
}

/// The result after applying a movement command.
pub enum RecordRowMoveResult<'conn, O, E> {
    /// The movement command succeeded.
    Updated(State<'conn, RecordKey>),
    /// The movement command failed, so the original row is returned along with some error context.
    Unchanged(State<'conn, O>, E),
}

impl<'conn, O: AsRecordKey, E> RecordRowMoveResult<'conn, O, E> {
    fn from_rowid(
        original: State<'conn, O>,
        candidate: Result<i64, E>,
    ) -> Result<Self, rusqlite::Error> {
        match candidate {
            Ok(row_id) => {
                original.update_citation_keys(row_id)?;
                Ok(RecordRowMoveResult::Updated(State::init(
                    original.tx,
                    RecordKey(row_id),
                )))
            }
            Err(e) => Ok(RecordRowMoveResult::Unchanged(original, e)),
        }
    }
}

pub enum SetActiveError {
    RowIdUndefined,
    DifferentCanonical(RemoteId),
}

impl<'conn, I: AsRecordKey> State<'conn, I> {
    pub(super) fn row_id(&self) -> i64 {
        self.id.row_id()
    }

    /// Forget whatever kind of row this is, and just replace it with a generic row in the
    /// 'Records' table.
    pub fn forget(self) -> State<'conn, RecordKey> {
        let row_id = self.row_id();
        State::init(self.tx, RecordKey(row_id))
    }

    /// Update the active row to a specific revision.
    ///
    /// The row-id must correspond to a row in the 'Records' table with a canonical id which is the same
    /// as the canonical id of this row.
    pub fn set_active(
        self,
        row_id: i64,
    ) -> Result<RecordRowMoveResult<'conn, I, SetActiveError>, rusqlite::Error> {
        let canonical = RemoteId::lookup(&self)?;

        // check if the row id corresponds to a row in the records table, and moreover that the
        // corresonding canonical id is the same
        let row_id_or_err =
            match <RemoteId as RecordsLookup<I>>::lookup_unchecked(&self.tx, row_id).optional()? {
                Some(target_canonical) if target_canonical == canonical => Ok(row_id),
                Some(other_canonical) => Err(SetActiveError::DifferentCanonical(other_canonical)),
                None => Err(SetActiveError::RowIdUndefined),
            };

        RecordRowMoveResult::from_rowid(self, row_id_or_err)
    }

    /// Update the active row to be the parent of this row.
    pub fn set_parent_as_active(
        self,
    ) -> Result<RecordRowMoveResult<'conn, I, ()>, rusqlite::Error> {
        let context = RecordContext::lookup(&self)?;
        RecordRowMoveResult::from_rowid(self, context.parent.ok_or(()))
    }

    /// Update the active row to be a child of this row.
    ///
    /// If `index` is none and there is a unique child, this method will succeed. Otherwise,
    /// attempt to set to the `nth` child, ordered from first to last, where negative indices count
    /// backwards.
    ///
    /// The returned index on error is the number of children.
    pub fn set_child_as_active(
        self,
        index: Option<isize>,
    ) -> Result<RecordRowMoveResult<'conn, I, usize>, rusqlite::Error> {
        let context = RecordContext::lookup(&self)?;
        match index {
            Some(idx) => {
                if idx >= 0 {
                    RecordRowMoveResult::from_rowid(
                        self,
                        context
                            .child_keys()
                            .nth(idx.abs_diff(0))
                            .ok_or_else(|| context.child_count()),
                    )
                } else {
                    RecordRowMoveResult::from_rowid(
                        self,
                        context
                            .child_keys()
                            .nth_back(idx.abs_diff(-1))
                            .ok_or_else(|| context.child_count()),
                    )
                }
            }
            None => RecordRowMoveResult::from_rowid(self, context.unique_child()),
        }
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

        Ok(State::init(self.tx, Missing {}))
    }

    /// Get the canonical [`RemoteId`].
    #[inline]
    pub fn canonical(&self) -> Result<RemoteId, rusqlite::Error> {
        debug!("Getting canonical identifier for '{}'.", self.row_id());
        RemoteId::lookup(self)
    }

    /// Get last modified time.
    #[inline]
    pub fn last_modified(&self) -> Result<DateTime<Local>, rusqlite::Error> {
        debug!("Getting last modified time for row '{}'.", self.row_id());
        DateTime::lookup(self)
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
}

/// The row data associated with a row in the `Records` table. The precise value depends on the
/// `variant` column.
#[derive(Debug)]
pub enum EntryDataOrReplacement {
    /// Entry data.
    Entry(RawEntryData),
    /// Deleted data.
    Deleted(Option<RemoteId>),
}

impl EntryDataOrReplacement {
    fn from_bytes_and_variant(data_bytes: Vec<u8>, variant: i64) -> Self {
        match variant {
            0 => Self::Entry(RawEntryData::from_byte_repr_unchecked(data_bytes)),
            1 => {
                let remote_id = if data_bytes.is_empty() {
                    None
                } else {
                    Some(RemoteId::from_string_unchecked(
                            data_bytes.try_into().expect("Invalid database: 'data' column for deleted row contains non-UTF8 blob data."),
                        ))
                };
                Self::Deleted(remote_id)
            }
            v => {
                panic!("Corrupted database: 'Records' table contains row with invalid variant {v}")
            }
        }
    }
}

#[derive(Debug)]
pub struct EntryRowData {
    pub data: RawEntryData,
    pub canonical: RemoteId,
    pub modified: DateTime<Local>,
}

#[derive(Debug)]
pub struct DeletedRowData {
    pub replacement: Option<RemoteId>,
    pub canonical: RemoteId,
    pub modified: DateTime<Local>,
}

#[derive(Debug)]
pub struct RecordRowData {
    /// The associated data.
    pub data: EntryDataOrReplacement,
    /// The canonical identifier.
    pub canonical: RemoteId,
    /// When the record was modified.
    pub modified: DateTime<Local>,
}

impl From<EntryRowData> for RecordRowData {
    fn from(
        EntryRowData {
            data,
            canonical,
            modified,
        }: EntryRowData,
    ) -> Self {
        Self {
            data: EntryDataOrReplacement::Entry(data),
            canonical,
            modified,
        }
    }
}

impl From<DeletedRowData> for RecordRowData {
    fn from(
        DeletedRowData {
            replacement,
            canonical,
            modified,
        }: DeletedRowData,
    ) -> Self {
        Self {
            data: EntryDataOrReplacement::Deleted(replacement),
            canonical,
            modified,
        }
    }
}

// TODO: remove this implementation
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

pub struct RecordContext {
    pub data: EntryDataOrReplacement,
    pub canonical: RemoteId,
    pub modified: DateTime<Local>,
    pub(super) parent: Option<i64>,
    pub(super) children: Vec<u8>,
}

impl RecordContext {
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
    fn unique_child(&self) -> Result<i64, usize> {
        let count = self.child_count();
        if count == 1 {
            Ok(self.child_keys().next().unwrap())
        } else {
            Err(count)
        }
    }
}

impl<'conn> State<'conn, EntryRecordKey> {
    /// Get the bibliographic data associated with this row.
    pub fn get_data(&self) -> Result<EntryRowData, rusqlite::Error> {
        debug!(
            "Retrieving 'Records' data associated with row '{}'",
            self.row_id()
        );
        EntryRowData::lookup(self)
    }

    /// Insert new data, preserving the old row as the parent row.
    pub fn modify(self, data: &RawEntryData) -> Result<Self, rusqlite::Error> {
        let new_key = self.replace_impl(data)?;
        Ok(Self::init(self.tx, EntryRecordKey(new_key)))
    }

    /// Replace this row with a deletion marker, preserving the old row as the parent row.
    pub fn soft_delete(
        self,
        replacement: &Option<RemoteId>,
    ) -> Result<State<'conn, DeletedRecordKey>, rusqlite::Error> {
        let new_key = self.replace_impl(replacement)?;
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
                    Ok(Some(RemoteId::lookup(self)?))
                }
            }
            None => {
                self.prepare("INSERT INTO CitationKeys (name, record_key) values (?1, ?2)")?
                    .execute((alias.name(), self.row_id()))?;
                Ok(None)
            }
        }
    }

    fn replace_impl<R: RecordsDataCol>(&self, data: &R) -> Result<i64, rusqlite::Error> {
        // read the current value of 'record_id' and 'children'
        let existing = RecordContext::lookup(self)?;

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
            .query_row((existing.canonical.name(), data.data_blob(), Local::now(), data.variant(), self.row_id()), |row| row.get(0))?;

        // update the `children` field with the existing records
        let mut new_children = existing.children;
        new_children.extend(new_key.to_le_bytes());
        self.prepare("UPDATE Records SET children = ?1 WHERE key = ?2")?
            .execute((new_children, self.row_id()))?;

        self.update_citation_keys(new_key)?;

        Ok(new_key)
    }
}

impl<'conn> State<'conn, RecordKey> {
    pub fn determine(self) -> Result<EntryOrDeletedRow<'conn>, rusqlite::Error> {
        let RecordRowData {
            data,
            modified,
            canonical,
        } = RecordRowData::lookup(&self)?;

        let row_id = self.row_id();

        Ok(match data {
            EntryDataOrReplacement::Entry(data) => EntryOrDeletedRow::Entry(
                EntryRowData {
                    data,
                    modified,
                    canonical,
                },
                State::init(self.tx, EntryRecordKey(row_id)),
            ),
            EntryDataOrReplacement::Deleted(replacement) => EntryOrDeletedRow::Deleted(
                DeletedRowData {
                    replacement,
                    modified,
                    canonical,
                },
                State::init(self.tx, DeletedRecordKey(row_id)),
            ),
        })
    }
}

/// A row in the 'Records' table which either exists or was deleted.
pub enum EntryOrDeletedRow<'conn> {
    Entry(EntryRowData, State<'conn, EntryRecordKey>),
    Deleted(DeletedRowData, State<'conn, DeletedRecordKey>),
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
