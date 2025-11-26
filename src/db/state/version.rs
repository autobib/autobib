use std::{fmt, str::FromStr};

use super::{ArbitraryData, CompleteRecordRow, InRecordsTable, RecordRow, State, Transaction};

/// A specific version of a record row.
///
/// The lifetime is tied to the transaction in which the version is guaranteed to be valid.
#[derive(Debug)]
pub struct Version<'tx, 'conn> {
    pub row: RecordRow<ArbitraryData>,
    pub(super) row_id: i64,
    pub(super) tx: &'tx Transaction<'conn>,
    parent_row_id: Option<i64>,
    child_row_ids: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct RevisionId(pub(super) i64);

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
    /// Determine the number of elements in the changelog to obtain an iteration bound.
    pub fn changelog_size(&self) -> rusqlite::Result<usize> {
        self.prepare("SELECT COUNT(*) FROM Records WHERE record_id = (SELECT record_id from Records WHERE key = ?1)")?
            .query_row((self.row_id(),), |row| row.get(0))
    }

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
            child_row_ids: row.children,
            tx,
            row_id,
        })
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
    pub fn num_children(&self) -> usize {
        self.child_iter().len()
    }

    /// Returns an iterator over the child rows, ordered from newest to oldest.
    pub fn child_iter(
        &self,
    ) -> impl DoubleEndedIterator<Item = rusqlite::Result<Version<'tx, 'conn>>> + ExactSizeIterator
    {
        self.child_row_ids
            .as_chunks()
            .0
            .iter()
            .map(|chunk| Version::init(self.tx, i64::from_le_bytes(*chunk)))
    }
}
