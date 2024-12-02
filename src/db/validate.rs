use std::{fmt, num::NonZero};

use rusqlite::types::ValueRef;

use super::{get_row_id, sql, Transaction};
use crate::{error::InvalidBytesError, logger::debug, RawRecordData, RecordId, RemoteId};

/// A possible fault that could occurr inside the database.
#[derive(Debug)]
pub enum DatabaseFault {
    /// A row has an invalid canonical id.
    RowHasInvalidCanonicalId(i64, String),
    /// A row does not correspond to a row in the `CitationKeys` table.
    DanglingRecord(i64, String),
    /// There are `NonZero<usize>` rows in the `CitationKeys` table which point to a `Records` row which does not exist.
    NullCitationKeys(NonZero<usize>),
    /// There was an underlying SQLite integrity error.
    IntegrityError(String),
    /// A row in the `Records` table contains invalid binary data.
    InvalidRecordData(i64, String, InvalidBytesError),
}

impl fmt::Display for DatabaseFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DatabaseFault::RowHasInvalidCanonicalId(row_id, name) => {
                write!(
                    f,
                    "Record row '{row_id}' contains record id '{name}' which is not a valid canonical id."
                )
            }
            DatabaseFault::DanglingRecord(row_id, name) => {
                write!(
                    f,
                    "Record row '{row_id}' with record id '{name}' does not have corresponding key in the `CitationKeys` table."
                )
            }
            DatabaseFault::NullCitationKeys(count) => {
                if count.get() == 1 {
                    write!(
                        f,
                        "A citation key references a record which does not exist in the database."
                    )
                } else {
                    write!(
                        f,
                        "There are {count} citation keys which reference records which do not exist in the database."
                    )
                }
            }
            DatabaseFault::IntegrityError(err) => write!(f, "Database integrity error: {err}"),
            DatabaseFault::InvalidRecordData(row_id, name, err) => write!(
                f,
                "Record row '{row_id}' with record id '{name}' has invalid binary data: {err}"
            ),
        }
    }
}

pub struct DatabaseValidator<'conn> {
    pub tx: Transaction<'conn>,
}

impl<'conn> DatabaseValidator<'conn> {
    pub fn into_tx(self) -> Transaction<'conn> {
        self.tx
    }

    /// Check the contents of the `Records` table for the following errors:
    /// 1. Invalid formats of canonical ids.
    /// 2. Records which do not correspond to any rows in the `CitationKeys` table.
    pub fn record_indexing(&self, faults: &mut Vec<DatabaseFault>) -> Result<(), rusqlite::Error> {
        debug!("Checking record indexing");
        let mut retriever = self.tx.prepare("SELECT * FROM Records")?;
        let mut rows = retriever.query([])?;

        while let Some(row) = rows.next()? {
            // first verify that we actually get a proper canonical id
            let row_id = row.get("key")?;
            let name: String = row.get("record_id")?;
            let canonical_id: RemoteId = match RecordId::from(name.as_ref()).try_into() {
                Ok(remote_id) => remote_id,
                Err(_) => {
                    faults.push(DatabaseFault::RowHasInvalidCanonicalId(row_id, name));
                    continue;
                }
            };

            // then check that the corresponding record is in the `CitationKeys` table
            if get_row_id(&self.tx, &canonical_id)?.is_none() {
                faults.push(DatabaseFault::DanglingRecord(row_id, name));
            }
        }
        Ok(())
    }

    /// Check the database for integrity issues.
    pub fn integrity(&self, faults: &mut Vec<DatabaseFault>) -> Result<(), rusqlite::Error> {
        debug!("Checking integrity");
        self.tx.pragma_query(None, "integrity_check", |row| {
            if !matches!(row.get_ref(0)?, ValueRef::Text(b"ok")) {
                let err: String = row.get(0)?;
                faults.push(DatabaseFault::IntegrityError(err));
            }
            Ok(())
        })
    }

    /// Check the `CitationKeys` table for foreign key constraint violations.
    pub fn invalid_citation_keys(
        &self,
        faults: &mut Vec<DatabaseFault>,
    ) -> Result<(), rusqlite::Error> {
        debug!("Checking citation key table consistency");
        let mut num_faults: usize = 0;

        // since `CitationKeys` is a `WITHOUT ROWID` table, `PRAGMA foreign_key_check;` does not
        // return meaningful information since it cannot provide a rowid for which the foreign key
        // constraint is violated. As a result, the best way for us to handle this is just to
        // return the number of violations.
        self.tx.pragma_query(None, "foreign_key_check", |_| {
            num_faults += 1;
            Ok(())
        })?;

        if let Some(nz) = NonZero::new(num_faults) {
            faults.push(DatabaseFault::NullCitationKeys(nz));
        }

        Ok(())
    }

    /// Validate binary data in the `Records` table.
    pub fn binary_data(&self, faults: &mut Vec<DatabaseFault>) -> Result<(), rusqlite::Error> {
        debug!("Checking binary data correctness");
        let mut retriever = self.tx.prepare(sql::get_all_record_data())?;
        let mut rows = retriever.query([])?;

        while let Some(row) = rows.next()? {
            if let Err(err) = RawRecordData::from_byte_repr(row.get("data")?) {
                faults.push(DatabaseFault::InvalidRecordData(
                    row.get("key")?,
                    row.get("record_id")?,
                    err,
                ));
            }
        }
        Ok(())
    }
}
