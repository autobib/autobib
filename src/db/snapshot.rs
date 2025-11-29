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

    /// Delete all 'orphaned' records.
    ///
    /// Usually these should not exist in the database, but sometimes it is useful to temporarily
    /// put the database in this state and then cleanup afterwards.
    pub fn prune_orphaned(&self) -> rusqlite::Result<()> {
        self.tx
            .prepare(
                "
WITH RECURSIVE ancestors AS (
    SELECT key, parent_key
    FROM Records
    WHERE key IN (SELECT record_key FROM Identifiers)

    UNION ALL

    SELECT r.key, r.parent_key
    FROM ancestors a
    INNER JOIN Records r ON a.parent_key = r.key
),
descendants AS (
    SELECT key FROM ancestors

    UNION

    SELECT r.key
    FROM Records r
    INNER JOIN descendants d ON r.parent_key = d.key
)
DELETE FROM Records WHERE key NOT IN (SELECT key FROM descendants);
",
            )?
            .execute([])?;
        Ok(())
    }

    /// Delete all inactive records.
    pub fn prune_all(&self) -> rusqlite::Result<()> {
        // delete everything which is not active. we don't need to set `parent_key = NULL` because
        // of the `ON DELETE SET NULL` foreign key constraint
        self.tx
            .prepare("DELETE FROM Records WHERE key NOT IN (SELECT record_key FROM Identifiers)")?
            .execute([])?;
        Ok(())
    }

    /// Prune all 'oudated' entries: that is, those which are not a descendent of a currently
    /// active entry.
    pub fn prune_outdated(&self) -> rusqlite::Result<()> {
        self.tx
            .prepare(
                "
WITH RECURSIVE descendants AS (
  SELECT DISTINCT record_key FROM Identifiers

  UNION ALL

  SELECT key
  FROM Records
  INNER JOIN descendants ON Records.parent_key = descendants.key
)
DELETE FROM Records WHERE key NOT IN (SELECT key FROM descendants);",
            )?
            .execute([])?;
        Ok(())
    }

    /// Prune all revisions which are not a descendent of a level `n` ancestor of an active
    /// revision.
    pub fn prune_outdated_keep(&self, retain: u32) -> rusqlite::Result<()> {
        self.tx
            .prepare(
                "
WITH RECURSIVE ancestors AS (
    SELECT key, parent_key, 0 as level
    FROM Records
    WHERE key IN (SELECT record_key FROM Identifiers)

    UNION ALL

    SELECT r.key, r.parent_key, a.level + 1
    FROM ancestors a
    INNER JOIN Records r ON a.parent_key = r.key
    WHERE a.level < ?1
),
descendants AS (
    SELECT key FROM ancestors

    UNION

    SELECT r.key
    FROM Records r
    INNER JOIN descendants d ON r.parent_key = d.key
)
DELETE FROM Records WHERE key NOT IN (SELECT key FROM descendants);
",
            )?
            .execute([retain])?;
        Ok(())
    }

    /// Check whether a specific revision is active.
    pub fn is_active(&self, rev_id: RevisionId) -> rusqlite::Result<bool> {
        self.tx
            .prepare("SELECT EXISTS (SELECT 1 FROM Identifiers WHERE record_key = ?1)")?
            .query_one([rev_id.0], |row| row.get(0))
    }

    /// Delete inactive void records with exactly one child.
    pub fn prune_void(&self) -> rusqlite::Result<()> {
        self.tx
            .prepare(
                "
DELETE FROM Records
WHERE variant = 2
  AND key NOT IN (SELECT record_key FROM Identifiers)
  AND (SELECT count(*) FROM Records AS r WHERE r.parent_key = Records.key LIMIT 2) = 1",
            )?
            .execute([])?;
        Ok(())
    }

    /// Delete inactive deleted records which have no children.
    pub fn prune_deleted(&self) -> rusqlite::Result<()> {
        // the `parent_key` is automatically set to null when the parent is deleted
        self.tx
            .prepare(
                "
DELETE FROM Records
WHERE variant = 1
  AND key NOT IN (SELECT record_key FROM Identifiers)
  AND NOT EXISTS (SELECT 1 FROM Records AS r WHERE r.parent_key = Records.key)",
            )?
            .execute([])?;

        Ok(())
    }

    /// Iterate over all active entries in the Records table, adding the revisions to the list
    /// which are later than the threshold date.
    pub fn rewind_all(&self, after: DateTime<Local>) -> rusqlite::Result<()> {
        let mut retriever = self
            .tx
            .prepare("SELECT record_id, key FROM Records WHERE key IN (SELECT record_key FROM Identifiers) AND modified > ?1")?;

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
                .prepare_cached("UPDATE Identifiers SET record_key = ?1 WHERE record_key = ?2")?
                .execute((new_row_id, row_id))?;
        }
        Ok(())
    }

    /// Iterate over all active entries in the Records table, adding the revisions to the list
    /// for which the provided closure returns true.
    pub fn filter_active_keys<F, T>(&self, mut f: F, buffer: &mut T) -> rusqlite::Result<()>
    where
        F: FnMut(RecordRow<ArbitraryDataRef<'_>, &'_ str>) -> bool,
        T: Extend<RevisionId>,
    {
        let mut retriever = self
            .tx
            .prepare("SELECT key, record_id, modified, data, variant FROM Records WHERE key IN (SELECT record_key FROM Identifiers)")?;

        let rows = retriever.query_map([], move |row| {
            let record_row = RecordRow::borrow_from_row_unchecked(row);
            let rev_id: RevisionId = row.get_unwrap("key");
            Ok(if f(record_row) { Some(rev_id) } else { None })
        })?;
        buffer.extend(rows.filter_map(|row| match row {
            Ok(Some(t)) => Some(t),
            // err is unreachable here because of the implementation in
            // query_map above, which panics immediately if there is an issue
            _ => None,
        }));
        Ok(())
    }

    /// Iterate over all names in the Identifiers table and apply the fallible closure
    /// `f` to each key. If an error is returned by the closure, it is immediately propagated and
    /// the function exits early.
    ///
    /// If `canonical` is true, only iterate over canonical keys.
    pub fn map_identifiers<E, F: FnMut(&str) -> Result<(), E>>(
        &self,
        canonical: bool,
        mut f: F,
    ) -> Result<(), SnapshotMapErr<E>> {
        let mut selector = if canonical {
            self.tx
                .prepare("SELECT record_id FROM Records WHERE key IN (SELECT record_key FROM Identifiers) AND variant = 0")?
        } else {
            self.tx.prepare("SELECT name FROM Identifiers INNER JOIN Records ON Identifiers.record_key = Records.key WHERE Records.variant = 0")?
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
