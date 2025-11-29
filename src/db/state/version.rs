use std::{fmt, str::FromStr};

use rusqlite::types::{FromSql, FromSqlError, ValueRef};

use super::{
    ArbitraryData, CompleteRecordRow, InRecordsTable, RecordRow, RecordRowDisplay, State,
    Transaction,
};

/// A specific version of a record row.
///
/// The lifetime is tied to the transaction in which the version is guaranteed to be valid.
#[derive(Debug)]
pub struct Version<'tx, 'conn> {
    pub row: RecordRow<ArbitraryData>,
    pub(in crate::db) row_id: i64,
    pub(super) tx: &'tx Transaction<'conn>,
    parent_row_id: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct RevisionId(pub(in crate::db) i64);

impl FromSql for RevisionId {
    fn column_result(value: ValueRef<'_>) -> Result<Self, FromSqlError> {
        if let ValueRef::Integer(row_id) = value {
            Ok(Self(row_id))
        } else {
            Err(FromSqlError::InvalidType)
        }
    }
}

impl fmt::Display for RevisionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rev {:0>4x}", self.0)
    }
}

impl FromStr for RevisionId {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        i64::from_str_radix(s, 16).map(RevisionId)
    }
}

/// Changelog implementation
impl<'conn, I: InRecordsTable> State<'conn, I> {
    /// Get the version associated with the row.
    pub fn current<'tx>(&'tx self) -> rusqlite::Result<Version<'tx, 'conn>> {
        Version::init(&self.tx, self.row_id())
    }
}

impl<'tx, 'conn> Version<'tx, 'conn> {
    fn init(tx: &'tx Transaction<'conn>, row_id: i64) -> rusqlite::Result<Self> {
        let row = CompleteRecordRow::load_unchecked(tx, row_id)?;
        Ok(Self {
            row: row.row,
            parent_row_id: row.parent,
            tx,
            row_id,
        })
    }

    fn new(
        tx: &'tx Transaction<'conn>,
        row_id: i64,
        row: CompleteRecordRow<super::ArbitraryData>,
    ) -> Self {
        Self {
            row: row.row,
            parent_row_id: row.parent,
            tx,
            row_id,
        }
    }

    pub fn is_deleted(&self) -> bool {
        matches!(self.row.data, ArbitraryData::Deleted(_))
    }

    pub fn is_entry(&self) -> bool {
        matches!(self.row.data, ArbitraryData::Entry(_))
    }

    pub fn is_void(&self) -> bool {
        matches!(self.row.data, ArbitraryData::Void)
    }

    /// Returns the parent row, if any.
    pub fn parent(&self) -> rusqlite::Result<Option<Self>> {
        match self.parent_row_id {
            Some(row_id) => Version::init(self.tx, row_id).map(Some),
            None => Ok(None),
        }
    }

    /// Returns the root version, or none.
    pub fn root(mut self, all: bool) -> rusqlite::Result<Self> {
        while let Some(parent) = self.parent()? {
            if parent.is_entry() || all {
                self = parent;
            } else {
                return Ok(self);
            }
        }
        Ok(self)
    }

    /// Get a printable form of the row-id, suitable for displaying to an end user.
    pub fn rev_id(&self) -> RevisionId {
        RevisionId(self.row_id)
    }

    /// The number of children.
    pub fn num_children(&self) -> rusqlite::Result<usize> {
        self.tx
            .prepare_cached("SELECT count(*) FROM Records WHERE parent_key = ?1")?
            .query_row([self.row_id], |row| row.get(0))
    }

    /// Returns whether or not the row has children.
    pub fn has_children(&self) -> rusqlite::Result<bool> {
        self.tx
            .prepare_cached("SELECT EXISTS (SELECT 1 FROM Records WHERE parent_key = ?1);")?
            .query_row([self.row_id], |row| row.get(0))
    }

    /// Apply a mutable closure to the data for every child, along with its row-id.
    ///
    /// The order in which the closure is applied is unspecified.
    pub(super) fn map_children<F>(&self, mut f: F) -> rusqlite::Result<()>
    where
        F: FnMut(CompleteRecordRow<ArbitraryData>, i64) -> rusqlite::Result<()>,
    {
        // it is better to not use an ORDER BY clause here
        // since the number of results is in the majority of cases extremely
        // low anyway, and vec sorting methods are ultra-optimized for small
        // vectors
        let mut stmt = self
            .tx
            .prepare_cached("SELECT key, record_id, modified, data, variant, parent_key FROM Records WHERE parent_key = ?1")?;

        for r in stmt.query_map([self.row_id], |row| {
            Ok((
                CompleteRecordRow::from_row_unchecked(row),
                row.get_unwrap("key"),
            ))
        })? {
            let (data, row_id) = r?;
            f(data, row_id)?;
        }

        Ok(())
    }

    /// Returns the children in an unspecified order.
    pub fn children(&self) -> rusqlite::Result<Vec<Self>> {
        let mut children = Vec::new();
        self.map_children(|ch, row_id| {
            children.push(Version::new(self.tx, row_id, ch));
            Ok(())
        })?;

        Ok(children)
    }

    pub fn display(&self, styled: bool) -> RecordRowDisplay<'_> {
        RecordRowDisplay::from_version(self, styled)
    }
}
