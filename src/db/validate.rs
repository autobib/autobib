mod find_cycles;

use std::{
    collections::{HashMap, HashSet},
    fmt,
    num::NonZero,
    str::FromStr,
};

use autobib_entry::{AccessError, Archive, DataError, v1::ArchivedEntryData};
use chrono::{DateTime, Local};
use rusqlite::types::ValueRef;

use super::{Tx, schema};
use crate::{AsKey, Identifier, Key, logger::debug};

/// A possible fault that could occurr inside the database.
#[derive(Debug)]
pub enum DatabaseFault {
    /// The `parent_rev` relationship in the 'Records' table contains a cycle.
    ContainsCycle(HashSet<i64>),
    /// A void record is not a root vertex.
    VoidIsNotRoot(i64),
    /// A void record does not have the minimal timestamp.
    VoidHasIncorrectTimestamp(i64, DateTime<Local>),
    /// A row has a parent key with modification later than the row modification time.
    ParentHasEarlierTimestamp(i64),
    /// A record-id in the 'Records' table has multiple corresponding trees.
    OrphanedNodes(String, u64),
    /// A record-id in the 'Records' table has multiple citation keys pointing
    IncorrectActiveRowCount(String, u64),
    /// The `parent_rev` is a row which does not exist.
    ParentKeyMissing(i64),
    /// A row has an invalid canonical id.
    RowHasInvalidCanonicalId(i64, String),
    /// A row has a canonical id which has not been normalized.
    RowHasNonNormalizedCanonicalId(i64, String, String),
    /// A row has an invalid canonical id.
    InvalidKey(String),
    /// A row has a canonical id which has not been normalized.
    NonNormalizedId(String, String),
    /// There are `NonZero<usize>` rows in the `Keys` table which point to a `Records` row which does not exist.
    NullKeys(NonZero<usize>),
    /// There was an underlying SQLite integrity error.
    IntegrityError(String),
    /// A row in the `Records` table contains invalid binary data.
    InvalidRecordDataFormat(i64, String, AccessError),
    /// A row in the `Records` table contains invalid binary data.
    InvalidRecordData(i64, String, DataError),
    /// A table is missing.
    MissingTable(String),
    /// A table has the incorrect schema.
    InvalidTableSchema(String, String),
}

impl fmt::Display for DatabaseFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContainsCycle(cycle) => {
                write!(
                    f,
                    "Records table contains a cycle! This cycle uses the following row-ids:"
                )?;
                for key in cycle {
                    write!(f, " -> ({key})")?;
                }
                Ok(())
            }
            Self::ParentHasEarlierTimestamp(row_id) => {
                write!(
                    f,
                    "Row {row_id} has a parent key with modification later than the row modification time."
                )
            }
            Self::OrphanedNodes(key, n) => {
                write!(
                    f,
                    "Record id '{key}' contains inaccessible revisions: {n} disjoint revision-trees found."
                )
            }
            Self::IncorrectActiveRowCount(key, n) => {
                write!(f, "Record id '{key}' contains {n} active rows; expected 1.")
            }
            Self::ParentKeyMissing(parent_row_id) => {
                write!(
                    f,
                    "Parent key '{parent_row_id}' is not a row in the Records table"
                )
            }
            Self::VoidIsNotRoot(id) => {
                write!(f, "Void record '{id}' is not a root vertex")
            }
            Self::VoidHasIncorrectTimestamp(id, when) => {
                write!(
                    f,
                    "Void record '{id}' contains incorrect timestamp '{when}'"
                )
            }
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
            Self::InvalidKey(name) => {
                write!(
                    f,
                    "Keys table contains record id '{name}' which is not a valid canonical id"
                )
            }
            Self::NonNormalizedId(name, expected) => {
                write!(
                    f,
                    "Keys table contains record id '{name}' which is not normalized: expected '{expected}'"
                )
            }
            Self::NullKeys(count) => {
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
            Self::InvalidRecordDataFormat(row_id, name, err) => write!(
                f,
                "Record row '{row_id}' with record id '{name}' has invalid binary data: {err}"
            ),
            Self::InvalidRecordData(row_id, name, err) => write!(
                f,
                "Record row '{row_id}' with record id '{name}' has data not in the standard format: {err}"
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
    tx: &Tx,
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
    pub tx: Tx<'conn>,
}

impl<'conn> DatabaseValidator<'conn> {
    pub fn into_tx(self) -> Tx<'conn> {
        self.tx
    }

    /// Check that all of the expected tables exist and have the correct schema.
    pub fn table_schema(&self, faults: &mut Vec<DatabaseFault>) -> Result<(), rusqlite::Error> {
        for (tbl_name, schema) in [
            ("Records", schema::records()),
            ("Keys", schema::keys()),
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
    /// 2. Records which do not correspond to any rows in the `Keys` table.
    pub fn record_indexing(&self, faults: &mut Vec<DatabaseFault>) -> Result<(), rusqlite::Error> {
        debug!("Checking record indexing");
        let mut retriever = self.tx.prepare("SELECT * FROM Records")?;
        let mut rows = retriever.query([])?;

        while let Some(row) = rows.next()? {
            // first verify that we actually get a proper canonical id
            let row_id = row.get("rev")?;
            let name: String = row.get("canonical")?;
            let canonical_id: Identifier = match Identifier::from_str(name.as_ref()) {
                Ok(id) => id,
                Err(_) => {
                    faults.push(DatabaseFault::RowHasInvalidCanonicalId(row_id, name));
                    continue;
                }
            };

            if name != canonical_id.as_key() {
                faults.push(DatabaseFault::RowHasNonNormalizedCanonicalId(
                    row_id,
                    name,
                    canonical_id.as_key().to_string(),
                ));
                continue;
            }
        }
        Ok(())
    }

    pub fn unique_tree_per_key(&self, faults: &mut Vec<DatabaseFault>) -> rusqlite::Result<()> {
        debug!("Checking for cycles");
        let mut key_parent_pairs: HashMap<i64, Option<i64>> = HashMap::new();
        let mut stmt = self.tx.prepare("SELECT rev, parent_rev FROM Records")?;

        for row in stmt.query_map([], |row| Ok((row.get("rev")?, row.get("parent_rev")?)))? {
            let (key, parent) = row?;
            key_parent_pairs.insert(key, parent);
        }

        find_cycles::detect_cycles(&key_parent_pairs, faults);

        debug!("Checking that each canonical contains a unique tree");
        let mut stmt = self.tx.prepare("SELECT canonical, count(*) as root_count FROM Records WHERE parent_rev IS NULL GROUP BY canonical HAVING count(*) != 1")?;

        for row in stmt.query_map([], |row| {
            Ok((
                row.get("canonical")?,
                row.get("root_count").map(i64::unsigned_abs)?,
            ))
        })? {
            let (key, n) = row?;
            faults.push(DatabaseFault::OrphanedNodes(key, n));
        }

        Ok(())
    }

    pub fn check_active_row_counts(&self, faults: &mut Vec<DatabaseFault>) -> rusqlite::Result<()> {
        debug!("Checking that each canonical id occurs at most once in the Keys table");
        let mut stmt = self.tx.prepare(
            "
SELECT
    canonical,
    count(DISTINCT rev) as active_row_count
FROM Records
WHERE rev IN (SELECT record_rev FROM Keys)
GROUP BY canonical
HAVING count(DISTINCT rev) != 1
",
        )?;

        for row in stmt.query_map([], |row| {
            Ok((
                row.get("canonical")?,
                row.get("active_row_count").map(i64::unsigned_abs)?,
            ))
        })? {
            let (key, n) = row?;
            faults.push(DatabaseFault::IncorrectActiveRowCount(key, n));
        }

        debug!("Checking that each canonical id occurs in the Keys table");
        let mut stmt = self.tx.prepare(
            "
SELECT DISTINCT
    canonical
FROM Records
WHERE canonical NOT IN (
    SELECT r.canonical
    FROM Records AS r
    WHERE r.rev IN (SELECT record_rev FROM Keys)
)
",
        )?;

        for row in stmt.query_map([], |row| row.get("canonical"))? {
            faults.push(DatabaseFault::IncorrectActiveRowCount(row?, 0));
        }

        Ok(())
    }

    pub fn void_correct_formatting(&self, faults: &mut Vec<DatabaseFault>) -> rusqlite::Result<()> {
        debug!("Checking that void records do not have parents");
        let mut stmt = self
            .tx
            .prepare("SELECT rev FROM Records WHERE variant = 2 AND parent_rev IS NOT NULL")?;

        for row in stmt.query_map([], |row| row.get("rev"))? {
            faults.push(DatabaseFault::VoidIsNotRoot(row?));
        }

        debug!("Checking that void records have correct timestamp");
        let mut stmt = self
            .tx
            .prepare("SELECT rev, modified FROM Records WHERE variant = 2 AND modified != ?1")?;

        for row in stmt.query_map([DateTime::<Local>::MIN_UTC], |row| {
            Ok((row.get("rev")?, row.get("modified")?))
        })? {
            let (id, stamp) = row?;
            faults.push(DatabaseFault::VoidHasIncorrectTimestamp(id, stamp));
        }

        Ok(())
    }

    pub fn monotonic_timestamps(&self, fauls: &mut Vec<DatabaseFault>) -> rusqlite::Result<()> {
        let mut stmt = self.tx.prepare(
            "
SELECT DISTINCT c.rev as child_rev
FROM Records c JOIN Records p ON c.parent_rev = p.rev
WHERE c.modified < p.modified",
        )?;

        for row in stmt.query_map([], |row| row.get("child_rev"))? {
            fauls.push(DatabaseFault::ParentHasEarlierTimestamp(row?));
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

    /// Check the `Keys` table for foreign key constraint violations.
    pub fn invalid_identifiers(
        &self,
        faults: &mut Vec<DatabaseFault>,
    ) -> Result<(), rusqlite::Error> {
        debug!("Checking 'Keys' table consistency");
        let mut num_faults: usize = 0;

        // since `Keys` is a `WITHOUT ROWID` table, `PRAGMA foreign_key_check;` does not
        // return meaningful information since it cannot provide a rowid for which the foreign key
        // constraint is violated. As a result, the best way for us to handle this is just to
        // return the number of violations.
        self.tx.pragma_query(None, "foreign_key_check", |_| {
            num_faults += 1;
            Ok(())
        })?;

        if let Some(nz) = NonZero::new(num_faults) {
            faults.push(DatabaseFault::NullKeys(nz));
        }

        debug!("Checking 'Keys' table for non-normalized identifiers");
        let mut retriever = self.tx.prepare("SELECT * FROM Keys")?;
        let mut rows = retriever.query([])?;

        while let Some(row) = rows.next()? {
            let name: String = row.get("name")?;

            let id: String = match Key::from(name.as_ref()).resolve(&()) {
                Ok(alias_or_id) => alias_or_id.into(),
                Err(_) => {
                    faults.push(DatabaseFault::InvalidKey(name));
                    continue;
                }
            };

            if name != id {
                faults.push(DatabaseFault::NonNormalizedId(name, id));
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
            .prepare("SELECT canonical, data FROM Records WHERE variant = 0")?;
        let mut rows = retriever.query([])?;

        while let Some(row) = rows.next()? {
            match ArchivedEntryData::load(row.get("data")?) {
                Ok(data) => {
                    use autobib_entry::EntryData;
                    if let Err(err) = data.validate_untrusted() {
                        faults.push(DatabaseFault::InvalidRecordData(
                            row.get("rev")?,
                            row.get("canonical")?,
                            err,
                        ));
                    }
                }
                Err(err) => faults.push(DatabaseFault::InvalidRecordDataFormat(
                    row.get("rev")?,
                    row.get("canonical")?,
                    err,
                )),
            }
        }

        Ok(())
    }
}
