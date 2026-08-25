use std::{fmt, str::FromStr};

use rusqlite::{
    ToSql,
    types::{FromSql, FromSqlError, ToSqlOutput, ValueRef},
};
use serde::Serialize;

use super::{ArbitraryData, HistRecord, InRecordsTable, RecordRowDisplay, State, Tx};
use crate::db::{
    select::{FnMutMap, SelectOneUnchecked, SelectStatement, stmt},
    state::WithRev,
};

/// A specific version of a record row.
///
/// The lifetime is tied to the transaction in which the version is guaranteed to be valid.
#[derive(Debug)]
pub struct Version<'tx, 'conn> {
    pub hist: HistRecord,
    pub(in crate::db) row_id: TxRevId,
    pub(super) tx: &'tx Tx<'conn>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RevId(pub(in crate::db) i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TxRevId(pub(in crate::db) RevId);

impl ToSql for RevId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl ToSql for TxRevId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for RevId {
    fn column_result(value: ValueRef<'_>) -> Result<Self, FromSqlError> {
        if let ValueRef::Integer(row_id) = value {
            Ok(Self(row_id))
        } else {
            Err(FromSqlError::InvalidType)
        }
    }
}

impl Serialize for RevId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(&self)
    }
}

impl fmt::Display for RevId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:0>4x}", self.0)
    }
}

impl fmt::Display for TxRevId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for RevId {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        i64::from_str_radix(s, 16).map(RevId)
    }
}

impl RevId {
    pub fn fmt_pretty(&self) -> impl fmt::Display {
        RevIdPretty(self)
    }
}

impl TxRevId {
    pub(in crate::db) fn new(row_id: i64) -> Self {
        Self(RevId(row_id))
    }

    pub fn rev_id(self) -> RevId {
        self.0
    }

    pub(in crate::db) fn row_id(self) -> i64 {
        self.0.0
    }
}

struct RevIdPretty<'a>(&'a RevId);

impl fmt::Display for RevIdPretty<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rev {}", self.0)
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
    fn init(tx: &'tx Tx<'conn>, row_id: TxRevId) -> rusqlite::Result<Self> {
        let hist = stmt::GetHist::select_one_unchecked(tx, row_id)?;
        Ok(Self { hist, tx, row_id })
    }

    fn new(tx: &'tx Tx<'conn>, row_id: TxRevId, hist: HistRecord) -> Self {
        Self { hist, tx, row_id }
    }

    pub fn is_deleted(&self) -> bool {
        matches!(self.hist.record.data, ArbitraryData::Deleted(_))
    }

    pub fn is_entry(&self) -> bool {
        matches!(self.hist.record.data, ArbitraryData::Entry(_))
    }

    pub fn is_void(&self) -> bool {
        matches!(self.hist.record.data, ArbitraryData::Void)
    }

    /// Returns the parent row, if any.
    pub fn parent(&self) -> rusqlite::Result<Option<Self>> {
        match self.hist.parent {
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

    /// DOC-TODO[Document that this returns the durable revision identifier, not the private
    /// transaction-valid identifier.]
    pub fn rev_id(&self) -> RevId {
        self.row_id.rev_id()
    }

    pub(in crate::db) fn tx_rev_id(&self) -> TxRevId {
        self.row_id
    }

    /// The number of children.
    pub fn num_children(&self) -> rusqlite::Result<usize> {
        self.tx
            .prepare_cached("SELECT count(*) FROM Records WHERE parent_rev = ?1")?
            .query_row([self.row_id], |row| row.get(0).map(isize::unsigned_abs))
    }

    /// Returns whether or not the row has children.
    pub fn has_children(&self) -> rusqlite::Result<bool> {
        self.tx
            .prepare_cached("SELECT EXISTS (SELECT 1 FROM Records WHERE parent_rev = ?1);")?
            .query_row([self.row_id], |row| row.get(0))
    }

    /// Apply a mutable closure to the data for every child, along with its row-id.
    ///
    /// The order in which the closure is applied is unspecified.
    pub(super) fn map_children<F>(&self, mut f: F) -> rusqlite::Result<()>
    where
        F: FnMut(WithRev<HistRecord, TxRevId>),
    {
        stmt::SelectChildren::select_map(
            self.tx,
            #[allow(clippy::unit_arg)]
            FnMutMap::new(|r| Ok(f(r))),
            self.row_id,
        )?;
        Ok(())
    }

    /// Returns the children in an unspecified order.
    pub fn children(&self) -> rusqlite::Result<Vec<Self>> {
        let mut children = Vec::new();
        self.map_children(
            |WithRev {
                 inner: ch,
                 rev: row_id,
             }| {
                children.push(Version::new(self.tx, row_id, ch));
            },
        )?;

        Ok(children)
    }

    pub fn display(&self, styled: bool) -> RecordRowDisplay<'_> {
        RecordRowDisplay::from_version(self, styled)
    }
}
