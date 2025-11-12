use chrono::{DateTime, Local};

use crate::{
    Alias, RawEntryData, RemoteId,
    db::{CitationKey, Constraint, RowId, flatten_constraint_violation, get_row_id},
    logger::debug,
};

use super::{Missing, State};

/// A row which is present in the 'Records' table, but with unknown type.
#[derive(Debug)]
pub struct RecordRow(pub(super) RowId);

/// A soft deletion marker row in the 'Records' table.
#[derive(Debug)]
pub struct DeletedRow(RowId);

/// An entry in the 'Records' table.
#[derive(Debug)]
pub struct EntryRow(pub(super) RowId);

/// States which correspond to a row in the 'Records' table.
pub trait InRecordsTable {
    fn row_id(&self) -> RowId;
}

impl InRecordsTable for EntryRow {
    fn row_id(&self) -> RowId {
        self.0
    }
}

impl InRecordsTable for DeletedRow {
    fn row_id(&self) -> RowId {
        self.0
    }
}
impl InRecordsTable for RecordRow {
    fn row_id(&self) -> RowId {
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

impl RecordsDataCol for RecordRowVariant {
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
trait RecordsLookup<I: InRecordsTable>: Sized {
    fn lookup<'conn>(state: &State<'conn, I>) -> Result<Self, rusqlite::Error>;
}

impl<I: InRecordsTable> RecordsLookup<I> for DateTime<Local> {
    fn lookup<'conn>(state: &State<'conn, I>) -> Result<Self, rusqlite::Error> {
        state
            .prepare("SELECT modified FROM Records WHERE key = ?1")?
            .query_row([state.row_id()], |row| row.get("modified"))
    }
}

impl<I: InRecordsTable> RecordsLookup<I> for RemoteId {
    fn lookup<'conn>(state: &State<'conn, I>) -> Result<Self, rusqlite::Error> {
        state
            .prepare("SELECT record_id FROM Records WHERE key = ?1")?
            .query_row([state.row_id()], |row| {
                row.get("record_id").map(Self::from_string_unchecked)
            })
    }
}

impl RecordsLookup<EntryRow> for EntryRowData {
    fn lookup<'conn>(state: &State<'conn, EntryRow>) -> Result<Self, rusqlite::Error> {
        state
            .prepare_cached(
                "SELECT record_id, modified, data, variant FROM Records WHERE key = ?1",
            )?
            .query_row([state.row_id()], |row| row.try_into())
    }
}

impl<I: InRecordsTable> RecordsLookup<I> for RecordContext {
    fn lookup<'conn>(state: &State<'conn, I>) -> Result<Self, rusqlite::Error> {
        state
            .prepare_cached("SELECT record_id, parent_key, children FROM Records WHERE key = ?1")?
            .query_row([state.row_id()], |row| {
                let record_id = RemoteId::from_string_unchecked(row.get("record_id")?);
                let parent = row.get("parent_key")?;
                let children = row.get("children")?;

                Ok(Self {
                    record_id,
                    parent,
                    children,
                })
            })
    }
}

impl<'conn, I: InRecordsTable> State<'conn, I> {
    fn row_id(&self) -> RowId {
        self.id.row_id()
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
    /// the [`RecordRow`] for which the provided `filter_map` does not return `None`.
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
enum RecordRowVariant {
    /// Variant `0`, which is regular data
    Entry(RawEntryData),
    /// Variant `1`, which is a row which has been deleted
    Deleted(Option<RemoteId>),
}

#[derive(Debug)]
pub struct EntryRowData {
    pub data: RawEntryData,
    pub modified: DateTime<Local>,
    pub canonical: RemoteId,
}

#[derive(Debug)]
pub struct DeletedRowData {
    pub replacement: Option<RemoteId>,
    pub canonical: RemoteId,
    pub modified: DateTime<Local>,
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

struct RecordContext {
    record_id: RemoteId,
    parent: Option<i64>,
    children: Vec<u8>,
}

impl RecordContext {
    /// Returns an iterator over all child keys in order of creation.
    fn child_keys(&self) -> impl Iterator<Item = i64> {
        self.children
            .chunks_exact(8)
            .map(|chunk| i64::from_le_bytes(chunk.try_into().unwrap()))
    }
}

impl<'conn> State<'conn, EntryRow> {
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
        Ok(Self::init(self.tx, EntryRow(new_key)))
    }

    /// Replace this row with a deletion marker, preserving the old row as the parent row.
    pub fn soft_delete(
        self,
        replacement: &Option<RemoteId>,
    ) -> Result<State<'conn, DeletedRow>, rusqlite::Error> {
        let new_key = self.replace_impl(replacement)?;
        Ok(State {
            tx: self.tx,
            id: DeletedRow(new_key),
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
                    let EntryRowData { canonical, .. } = EntryRowData::lookup(self)?;
                    Ok(Some(canonical))
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
            .query_row((existing.record_id.name(), data.data_blob(), Local::now(), data.variant(), self.row_id()), |row| row.get(0))?;

        // update the `children` field with the existing records
        let mut new_children = existing.children;
        new_children.extend(new_key.to_le_bytes());
        self.prepare("UPDATE Records SET children = ?1 WHERE key = ?2")?
            .execute((new_children, self.row_id()))?;

        // update the `CitationKeys` table values
        self.prepare("UPDATE CitationKeys SET record_key = ?1 WHERE record_key = ?2")?
            .execute((new_key, self.row_id()))?;

        Ok(new_key)
    }
}

impl<'conn> State<'conn, RecordRow> {
    pub fn determine(self) -> Result<EntryOrDeletedRow<'conn>, rusqlite::Error> {
        /// Helper function so that rustfmt has an easier time
        #[inline]
        fn convert(
            row: &rusqlite::Row<'_>,
        ) -> Result<(RecordRowVariant, DateTime<Local>, RemoteId), rusqlite::Error> {
            let variant: i64 = row.get("variant")?;
            let data = match variant {
                0 => RecordRowVariant::Entry(RawEntryData::from_byte_repr_unchecked(
                    row.get("data")?,
                )),
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
                    panic!(
                        "Corrupted database: 'Records' table contains row with invalid variant {v}"
                    )
                }
            };
            let canonical = RemoteId::from_string_unchecked(row.get("record_id")?);
            let modified = row.get("modified")?;
            Ok((data, modified, canonical))
        }

        let row_id = self.row_id();

        let (variant, modified, canonical) = self
            .prepare_cached(
                "SELECT record_id, modified, data, variant FROM Records WHERE key = ?1",
            )?
            .query_row([row_id], convert)?;

        Ok(match variant {
            RecordRowVariant::Entry(data) => EntryOrDeletedRow::Entry(
                EntryRowData {
                    data,
                    modified,
                    canonical,
                },
                State::init(self.tx, EntryRow(row_id)),
            ),
            RecordRowVariant::Deleted(replacement) => EntryOrDeletedRow::Deleted(
                DeletedRowData {
                    replacement,
                    modified,
                    canonical,
                },
                State::init(self.tx, DeletedRow(row_id)),
            ),
        })
    }
}

/// A row in the 'Records' table which either exists or was deleted.
pub enum EntryOrDeletedRow<'conn> {
    Entry(EntryRowData, State<'conn, EntryRow>),
    Deleted(DeletedRowData, State<'conn, DeletedRow>),
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
