use chrono::Local;

use super::{EntryRecordKey, NotEntry, NullRecordRow, State};
use crate::{RawEntryData, RemoteId, db::CitationKey, entry::EntryData, logger::debug};

/// Types which know how to insert new data.
///
/// The precise behaviour depends on the implementation. For data which does not exist in the
/// database, this adds a new row to the 'Records' table, and for data which is already present,
/// this adds a new child vertex with the corresponding data.
pub trait RecordsInsert<'conn> {
    fn insert(
        self,
        data: &RawEntryData,
        canonical: &RemoteId,
    ) -> Result<State<'conn, EntryRecordKey>, rusqlite::Error>;
}

impl<'conn> RecordsInsert<'conn> for State<'conn, Missing> {
    fn insert(
        self,
        data: &RawEntryData,
        canonical: &RemoteId,
    ) -> Result<State<'conn, EntryRecordKey>, rusqlite::Error> {
        self.insert_new(data, canonical)
    }
}

impl<'conn, I: NotEntry> RecordsInsert<'conn> for State<'conn, I> {
    fn insert(
        self,
        data: &RawEntryData,
        _: &RemoteId,
    ) -> Result<State<'conn, EntryRecordKey>, rusqlite::Error> {
        self.reinsert(data)
    }
}

impl<'conn> RecordsInsert<'conn> for State<'conn, EntryRecordKey> {
    fn insert(self, data: &RawEntryData, _: &RemoteId) -> Result<Self, rusqlite::Error> {
        self.modify(data)
    }
}

/// A database id which is missing.
#[derive(Debug)]
pub struct Missing;

impl<'conn> State<'conn, Missing> {
    /// Set a null row, converting into a [`NullRecordRow`].
    pub fn set_null(
        self,
        remote_id: &RemoteId,
    ) -> Result<State<'conn, NullRecordRow>, rusqlite::Error> {
        let row_id: i64 = {
            let mut setter = self.prepare_cached("INSERT OR REPLACE INTO NullRecords (record_id, attempted) values (?1, ?2) RETURNING rowid")?;
            let cache_time = Local::now();
            setter.query_row((remote_id.name(), cache_time), |row| row.get(0))?
        };

        Ok(State::init(self.tx, NullRecordRow(row_id)))
    }

    /// Create the row.
    ///
    /// # Safety
    /// The 'canonical' remote id must be present in the provided `refs` iterator.
    pub(crate) fn insert_with_refs<'a, R: Iterator<Item = &'a RemoteId>>(
        self,
        data: &RawEntryData,
        canonical: &RemoteId,
        refs: R,
    ) -> Result<State<'conn, EntryRecordKey>, rusqlite::Error> {
        debug!("Inserting data for canonical id '{canonical}'");
        let row_id: i64 = self.prepare_cached("INSERT OR ABORT INTO Records (record_id, data, modified) values (?1, ?2, ?3) RETURNING key")?.query_row(
            (canonical.name(), data.to_byte_repr(), &Local::now()),
            |row| row.get(0),
        )?;
        let row = State::init(self.tx, EntryRecordKey(row_id));
        row.add_refs(refs)?;
        Ok(row)
    }

    /// A convenience wrapper around [`insert`](Self::insert) which first converts any type which
    /// implements [`EntryData`] into a [`RawEntryData`].
    pub fn insert_entry_data<D: EntryData>(
        self,
        data: &D,
        canonical: &RemoteId,
    ) -> Result<State<'conn, EntryRecordKey>, rusqlite::Error> {
        let raw_record_data = RawEntryData::from_entry_data(data);
        self.insert_new(&raw_record_data, canonical)
    }

    /// Create the row and also insert a link in the `CitationKeys` table.
    pub fn insert_new(
        self,
        data: &RawEntryData,
        canonical: &RemoteId,
    ) -> Result<State<'conn, EntryRecordKey>, rusqlite::Error> {
        // SAFETY: 'canonical' is passed as a ref.
        let row = self.insert_with_refs(data, canonical, std::iter::once(canonical))?;
        Ok(row)
    }
}
