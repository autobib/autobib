use std::{error, fmt};

use chrono::{DateTime, Local};
use rusqlite::types::ValueRef;

use crate::{db::state::create_rewind_target, logger::info};

use super::{
    Transaction,
    state::{ArbitraryDataRef, RecordRow, RevisionId},
};

pub struct Snapshot<'conn> {
    pub(super) tx: Transaction<'conn>,
}

#[derive(Debug)]
pub enum SnapshotMapErr<E> {
    CallbackFailed(E),
    DatabaseError(rusqlite::Error),
}

impl<E> From<rusqlite::Error> for SnapshotMapErr<E> {
    fn from(err: rusqlite::Error) -> Self {
        Self::DatabaseError(err)
    }
}

impl<E: fmt::Display> fmt::Display for SnapshotMapErr<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CallbackFailed(error) => error.fmt(f),
            Self::DatabaseError(error) => error.fmt(f),
        }
    }
}

impl<E: error::Error> error::Error for SnapshotMapErr<E> {}

impl<'conn> Snapshot<'conn> {
    /// Commit the changes made in this snapshot.
    pub fn commit(self) -> rusqlite::Result<()> {
        self.tx.commit()
    }

    /// Iterate over all entries in the Records table and apply the fallible closure to the data
    /// for each key. If an error is returned by the closure, it is immediately propagated and
    /// the function exits early.
    pub fn map_history<E, F>(&self, limit: Option<u32>, mut f: F) -> Result<(), SnapshotMapErr<E>>
    where
        F: FnMut(RecordRow<ArbitraryDataRef<'_>, &'_ str>, RevisionId) -> Result<(), E>,
    {
        // SQLite uses `-1` to indicate no limit
        let limit: i64 = limit.map(Into::into).unwrap_or(-1);
        let mut retriever = self
            .tx
            .prepare("SELECT key, record_id, modified, data, variant FROM Records WHERE variant != 2 ORDER BY modified DESC LIMIT ?1")?;

        let mut rows = retriever.query([limit])?;
        while let Some(row) = rows.next()? {
            let record_row = RecordRow::borrow_from_row_unchecked(row);
            let rev_id = row.get_unwrap("key");
            f(record_row, rev_id).map_err(SnapshotMapErr::CallbackFailed)?;
        }
        Ok(())
    }

    /// Iterate over all active entries in the Records table, adding the revisions to the todolist
    /// which are later than the threshold date.
    pub fn rewind_all(&self, after: DateTime<Local>) -> rusqlite::Result<()> {
        let mut retriever = self
            .tx
            .prepare("SELECT record_id, key FROM Records WHERE key IN (SELECT record_key FROM CitationKeys) AND modified > ?1")?;

        let mut outdated: Vec<(String, i64)> = Vec::new();

        for key in retriever.query_map([after], |row| {
            Ok((row.get_unwrap("record_id"), row.get_unwrap("key")))
        })? {
            outdated.push(key?);
        }

        for (canonical, row_id) in outdated {
            let new_row_id = create_rewind_target(&self.tx, &canonical, after)?;
            info!("Rewinding '{canonical}' from rev {row_id:0>4x} to rev {new_row_id:0>4x}");
            self.tx
                .prepare_cached("UPDATE CitationKeys SET record_key = ?1 WHERE record_key = ?2")?
                .execute((new_row_id, row_id))?;
        }
        Ok(())
    }

    /// Iterate over all active entries in the Records table, adding the revisions to the todolist
    /// for which the provided closure returns true.
    pub fn filter_active_keys<F, T>(&self, mut f: F, todolist: &mut T) -> rusqlite::Result<()>
    where
        F: FnMut(RecordRow<ArbitraryDataRef<'_>, &'_ str>) -> bool,
        T: Extend<RevisionId>,
    {
        let mut retriever = self
            .tx
            .prepare("SELECT key, record_id, modified, data, variant FROM Records WHERE key IN (SELECT record_key FROM CitationKeys)")?;

        let rows = retriever.query_map([], move |row| {
            let record_row = RecordRow::borrow_from_row_unchecked(row);
            let rev_id: RevisionId = row.get_unwrap("key");
            Ok(if f(record_row) { Some(rev_id) } else { None })
        })?;
        todolist.extend(rows.filter_map(|row| match row {
            Ok(Some(t)) => Some(t),
            // err is unreachable here because of the implementation in
            // query_map above, which panics immediately if there is an issue
            _ => None,
        }));
        Ok(())
    }

    /// Iterate over all names in the CitationKeys table and apply the fallible closure
    /// `f` to each key. If an error is returned by the closure, it is immediately propagated and
    /// the function exits early.
    ///
    /// If `canonical` is true, only iterate over canonical keys.
    pub fn map_citation_keys<E, F: FnMut(&str) -> Result<(), E>>(
        &self,
        canonical: bool,
        mut f: F,
    ) -> Result<(), SnapshotMapErr<E>> {
        let mut selector = if canonical {
            self.tx
                .prepare("SELECT record_id FROM Records WHERE key IN (SELECT record_key FROM CitationKeys) AND variant = 0")?
        } else {
            self.tx.prepare("SELECT name FROM CitationKeys INNER JOIN Records ON CitationKeys.record_key = Records.key WHERE Records.variant = 0")?
        };

        let mut rows = selector.query([])?;
        while let Some(row) = rows.next()? {
            if let ValueRef::Text(bytes) = row.get_ref_unwrap(0) {
                // SAFETY: the underlying data is always valid utf-8
                f(std::str::from_utf8(bytes).unwrap()).map_err(SnapshotMapErr::CallbackFailed)?;
            }
        }

        Ok(())
    }
}
