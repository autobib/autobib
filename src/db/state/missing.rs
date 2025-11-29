use chrono::Local;

use super::{IsEntry, IsNull, NotEntry, State};
use crate::{RawEntryData, RemoteId, db::Identifier, entry::EntryData, logger::debug};

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
    ) -> Result<State<'conn, IsEntry>, rusqlite::Error>;
}

impl<'conn> RecordsInsert<'conn> for State<'conn, IsMissing> {
    fn insert(
        self,
        data: &RawEntryData,
        canonical: &RemoteId,
    ) -> Result<State<'conn, IsEntry>, rusqlite::Error> {
        self.insert_new(data, canonical)
    }
}

impl<'conn, I: NotEntry> RecordsInsert<'conn> for State<'conn, I> {
    fn insert(
        self,
        data: &RawEntryData,
        _: &RemoteId,
    ) -> Result<State<'conn, IsEntry>, rusqlite::Error> {
        self.reinsert(data)
    }
}

impl<'conn> RecordsInsert<'conn> for State<'conn, IsEntry> {
    fn insert(self, data: &RawEntryData, _: &RemoteId) -> Result<Self, rusqlite::Error> {
        self.modify(data)
    }
}

/// A database id which is missing.
#[derive(Debug)]
pub struct IsMissing;

impl<'conn> State<'conn, IsMissing> {
    /// Set a null row, converting into the [`IsNull`] state.
    pub fn set_null(self, remote_id: &RemoteId) -> Result<State<'conn, IsNull>, rusqlite::Error> {
        let row_id: i64 = {
            let mut setter = self.prepare_cached("INSERT OR REPLACE INTO NullRecords (record_id, attempted) values (?1, ?2) RETURNING rowid")?;
            let cache_time = Local::now();
            setter.query_row((remote_id.name(), cache_time), |row| row.get(0))?
        };

        Ok(State::init(self.tx, IsNull(row_id)))
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
    ) -> Result<State<'conn, IsEntry>, rusqlite::Error> {
        debug!("Inserting data for canonical id '{canonical}'");
        let row_id: i64 = self.prepare_cached("INSERT OR ABORT INTO Records (record_id, data, modified) values (?1, ?2, ?3) RETURNING key")?.query_row(
            (canonical.name(), data.to_byte_repr(), &Local::now()),
            |row| row.get(0),
        )?;
        let row = State::init(self.tx, IsEntry(row_id));
        row.add_refs(refs)?;
        Ok(row)
    }

    /// A convenience wrapper around [`insert`](Self::insert) which first converts any type which
    /// implements [`EntryData`] into a [`RawEntryData`].
    pub fn insert_entry_data<D: EntryData>(
        self,
        data: &D,
        canonical: &RemoteId,
    ) -> Result<State<'conn, IsEntry>, rusqlite::Error> {
        let raw_record_data = RawEntryData::from_entry_data(data);
        self.insert_new(&raw_record_data, canonical)
    }

    /// Create the row and also insert a link in the `Identifiers` table.
    pub fn insert_new(
        self,
        data: &RawEntryData,
        canonical: &RemoteId,
    ) -> Result<State<'conn, IsEntry>, rusqlite::Error> {
        // SAFETY: 'canonical' is passed as a ref.
        let row = self.insert_with_refs(data, canonical, std::iter::once(canonical))?;
        Ok(row)
    }
}
