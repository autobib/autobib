use chrono::Local;

use super::{DatabaseId, NullRecordRow, RecordRow, State};
use crate::{
    db::{sql, CitationKey},
    logger::debug,
    RawRecordData, RemoteId,
};

/// A database id which is missing.
#[derive(Debug)]
pub struct Missing;

impl DatabaseId for Missing {}

impl<'conn> State<'conn, Missing> {
    /// Set a null row, converting into a [`NullRecordRow`].
    pub fn set_null(
        self,
        remote_id: &RemoteId,
    ) -> Result<State<'conn, NullRecordRow>, rusqlite::Error> {
        {
            let mut setter = self.prepare_cached(sql::set_cached_null())?;
            let cache_time = Local::now();
            setter.execute((remote_id.name(), cache_time))?;
        }
        // SAFETY: the `set_cached_null` statement is an INSERT.
        Ok(unsafe { self.into_last_insert() })
    }

    /// Create the row, converting into a [`RecordRow`].
    ///
    /// # Safety
    /// The 'canonical' remote id must be present in the provided `refs` iterator.
    pub(crate) unsafe fn insert_with_refs<'a, R: Iterator<Item = &'a RemoteId>>(
        self,
        data: &RawRecordData,
        canonical: &RemoteId,
        refs: R,
    ) -> Result<State<'conn, RecordRow>, rusqlite::Error> {
        debug!("Inserting data for canonical id '{canonical}'");
        self.prepare_cached(sql::set_cached_data())?.execute((
            canonical.name(),
            data.to_byte_repr(),
            &Local::now(),
        ))?;
        // SAFETY: the `set_cached_data` statement is an INSERT.
        let row = unsafe { self.into_last_insert() };
        row.add_refs(refs)?;
        Ok(row)
    }

    /// Create the row and also insert a link in the `CitationKeys` table, converting into a [`RecordRow`].
    pub fn insert(
        self,
        data: &RawRecordData,
        canonical: &RemoteId,
    ) -> Result<State<'conn, RecordRow>, rusqlite::Error> {
        // SAFETY: 'canonical' is passed as a ref.
        let row = unsafe { self.insert_with_refs(data, canonical, std::iter::once(canonical))? };
        Ok(row)
    }
}
