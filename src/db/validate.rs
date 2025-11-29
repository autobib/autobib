use std::{fmt, num::NonZero, str::FromStr};

use rusqlite::types::ValueRef;

use super::{Transaction, get_row_id, schema};
use crate::{
    Identifier, RawEntryData, RecordId, RemoteId, error::InvalidBytesError, logger::debug,
};

/// A possible fault that could occurr inside the database.
#[derive(Debug)]
pub enum DatabaseFault {
    /// A row has an invalid canonical id.
    RowHasInvalidCanonicalId(i64, String),
    /// A row has a canonical id which has not been normalized.
    RowHasNonNormalizedCanonicalId(i64, String, String),
    /// A row has an invalid canonical id.
    InvalidIdentifier(String),
    /// A row has a canonical id which has not been normalized.
    NonNormalizedIdentifier(String, String),
    /// A row does not correspond to a row in the `Identifiers` table.
    DanglingRecord(i64, String),
    /// There are `NonZero<usize>` rows in the `Identifiers` table which point to a `Records` row which does not exist.
    NullIdentifiers(NonZero<usize>),
    /// There was an underlying SQLite integrity error.
    IntegrityError(String),
    /// A row in the `Records` table contains invalid binary data.
    InvalidRecordData(i64, String, InvalidBytesError),
    /// A table is missing.
    MissingTable(String),
    /// A table has the incorrect schema.
    InvalidTableSchema(String, String),
}

impl fmt::Display for DatabaseFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RowHasInvalidCanonicalId(row_id, name) => {
                write!(
                    f,
                    "Record row '{row_id}' contains record id '{name}' which is not a valid canonical id"
                )
            }
            Self::RowHasNonNormalizedCanonicalId(row_id, name, expected) => {
                write!(
                    f,
                    "Record row '{row_id}' contains record id '{name}' which is not normalized: expected '{expected}'"
                )
            }
            Self::InvalidIdentifier(name) => {
                write!(
                    f,
                    "Identifiers table contains record id '{name}' which is not a valid canonical id"
                )
            }
            Self::NonNormalizedIdentifier(name, expected) => {
                write!(
                    f,
                    "Identifiers table contains record id '{name}' which is not normalized: expected '{expected}'"
                )
            }
            Self::DanglingRecord(row_id, name) => {
                write!(
                    f,
                    "Record row '{row_id}' with record id '{name}' does not have corresponding key in the `Identifiers` table."
                )
            }
            Self::NullIdentifiers(count) => {
                if count.get() == 1 {
                    write!(
                        f,
                        "An identifier references a record which does not exist in the database."
                    )
                } else {
                    write!(
                        f,
                        "There are {count} identifiers which reference records which do not exist in the database."
                    )
                }
            }
            Self::IntegrityError(err) => write!(f, "Database integrity error: {err}"),
            Self::InvalidRecordData(row_id, name, err) => write!(
                f,
                "Record row '{row_id}' with record id '{name}' has invalid binary data: {err}"
            ),
            Self::MissingTable(table_name) => write!(f, "Missing table '{table_name}'"),
            Self::InvalidTableSchema(table_name, table_schema) => write!(
                f,
                "Table '{table_name}' has invalid schema:\n{table_schema}",
            ),
        }
    }
}

/// Validate the schema of an existing table, or return an appropriate error.
pub fn check_table_schema(
    tx: &Transaction,
    table_name: &str,
    expected_schema: &str,
) -> Result<Option<DatabaseFault>, rusqlite::Error> {
    let mut table_selector = tx.prepare("SELECT sql FROM sqlite_schema WHERE name = ?1")?;
    let mut record_rows = table_selector.query([table_name])?;
    match record_rows.next()? {
        Some(row) => {
            let table_schema: String = row.get("sql")?;
            if table_schema == expected_schema {
                Ok(None)
            } else {
                Ok(Some(DatabaseFault::InvalidTableSchema(
                    table_name.into(),
                    table_schema,
                )))
            }
        }
        None => Ok(Some(DatabaseFault::MissingTable(table_name.into()))),
    }
}

pub struct DatabaseValidator<'conn> {
    pub tx: Transaction<'conn>,
}

impl<'conn> DatabaseValidator<'conn> {
    pub fn into_tx(self) -> Transaction<'conn> {
        self.tx
    }

    /// Check that all of the expected tables exist and have the correct schema.
    pub fn table_schema(&self, faults: &mut Vec<DatabaseFault>) -> Result<(), rusqlite::Error> {
        for (tbl_name, schema) in [
            ("Records", schema::records()),
            ("Identifiers", schema::identifiers()),
            ("NullRecords", schema::null_records()),
        ] {
            debug!("Checking schema for table '{tbl_name}'.");
            if let Some(fault) = check_table_schema(&self.tx, tbl_name, schema)? {
                faults.push(fault);
            }
        }

        Ok(())
    }

    /// Check the contents of the `Records` table for the following errors:
    /// 1. Invalid formats of canonical ids.
    /// 2. Records which do not correspond to any rows in the `Identifiers` table.
    pub fn record_indexing(&self, faults: &mut Vec<DatabaseFault>) -> Result<(), rusqlite::Error> {
        debug!("Checking record indexing");
        let mut retriever = self.tx.prepare("SELECT * FROM Records")?;
        let mut rows = retriever.query([])?;

        while let Some(row) = rows.next()? {
            // first verify that we actually get a proper canonical id
            let row_id = row.get("key")?;
            let name: String = row.get("record_id")?;
            let canonical_id: RemoteId = match RemoteId::from_str(name.as_ref()) {
                Ok(remote_id) => remote_id,
                Err(_) => {
                    faults.push(DatabaseFault::RowHasInvalidCanonicalId(row_id, name));
                    continue;
                }
            };

            if name != canonical_id.name() {
                faults.push(DatabaseFault::RowHasNonNormalizedCanonicalId(
                    row_id,
                    name,
                    canonical_id.name().to_string(),
                ));
                continue;
            }

            // then check that the corresponding record is in the `Identifiers` table
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

    /// Check the `Identifiers` table for foreign key constraint violations.
    pub fn invalid_identifiers(
        &self,
        faults: &mut Vec<DatabaseFault>,
    ) -> Result<(), rusqlite::Error> {
        debug!("Checking 'Identifiers' table consistency");
        let mut num_faults: usize = 0;

        // since `Identifiers` is a `WITHOUT ROWID` table, `PRAGMA foreign_key_check;` does not
        // return meaningful information since it cannot provide a rowid for which the foreign key
        // constraint is violated. As a result, the best way for us to handle this is just to
        // return the number of violations.
        self.tx.pragma_query(None, "foreign_key_check", |_| {
            num_faults += 1;
            Ok(())
        })?;

        if let Some(nz) = NonZero::new(num_faults) {
            faults.push(DatabaseFault::NullIdentifiers(nz));
        }

        debug!("Checking 'Identifiers' table for non-normalized identifiers");
        let mut retriever = self.tx.prepare("SELECT * FROM Identifiers")?;
        let mut rows = retriever.query([])?;

        while let Some(row) = rows.next()? {
            let name: String = row.get("name")?;

            let id: String = match RecordId::from(name.as_ref()).resolve(&()) {
                Ok(alias_or_remote_id) => alias_or_remote_id.into(),
                Err(_) => {
                    faults.push(DatabaseFault::InvalidIdentifier(name));
                    continue;
                }
            };

            if name != id {
                faults.push(DatabaseFault::NonNormalizedIdentifier(name, id));
                continue;
            }
        }

        Ok(())
    }

    /// Validate binary data in the `Records` table.
    pub fn binary_data(&self, faults: &mut Vec<DatabaseFault>) -> Result<(), rusqlite::Error> {
        debug!("Checking binary data correctness");
        let mut retriever = self
            .tx
            .prepare("SELECT record_id, data FROM Records WHERE variant = 0")?;
        let mut rows = retriever.query([])?;

        while let Some(row) = rows.next()? {
            if let Err(err) = RawEntryData::<Vec<u8>>::from_byte_repr(row.get("data")?) {
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
