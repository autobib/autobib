use chrono::Local;
use log::debug;

use super::{DatabaseId, NullRecordRow, RecordRow, State};
use crate::{
    db::{sql, CitationKey},
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
    pub fn insert(
        self,
        data: &RawRecordData,
        canonical: &RemoteId,
    ) -> Result<State<'conn, RecordRow>, rusqlite::Error> {
        debug!("Inserting data for canonical id '{canonical}'");
        self.prepare_cached(sql::set_cached_data())?.execute((
            canonical.name(),
            data.to_byte_repr(),
            &Local::now(),
        ))?;
        // SAFETY: the `set_cached_data` statement is an INSERT.
        Ok(unsafe { self.into_last_insert() })
    }

    /// Create the row and also insert a link in the `CitationKeys` table, converting into a [`RecordRow`].
    pub fn insert_and_ref(
        self,
        data: &RawRecordData,
        canonical: &RemoteId,
    ) -> Result<State<'conn, RecordRow>, rusqlite::Error> {
        let row = self.insert(data, canonical)?;
        row.add_refs(std::iter::once(canonical))?;
        Ok(row)
    }
}
