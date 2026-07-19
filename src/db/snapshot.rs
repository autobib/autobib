use std::{convert::Infallible, error, fmt, str::from_utf8};

use chrono::{DateTime, Local};
use rusqlite::types::ValueRef;

use crate::{
    db::{AsKey, state::create_rewind_target},
    logger::info,
    record::Identifier,
};

use super::{
    Tx,
    state::{ArbitraryDataRef, Record, RevisionId},
};

pub struct Snapshot<'conn> {
    pub(super) tx: Tx<'conn>,
}

#[derive(Debug)]
pub enum SnapshotMapErr<E> {
    CallbackFailed(E),
    DatabaseError(rusqlite::Error),
}

impl From<SnapshotMapErr<Infallible>> for rusqlite::Error {
    fn from(value: SnapshotMapErr<Infallible>) -> Self {
        let SnapshotMapErr::DatabaseError(err) = value;
        err
    }
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
        F: FnMut(Record<ArbitraryDataRef<'_>, &'_ str>, RevisionId) -> Result<(), E>,
    {
        // SQLite uses `-1` to indicate no limit
        let limit: i64 = limit.map(Into::into).unwrap_or(-1);
        let mut retriever = self
            .tx
            .prepare("SELECT rev, canonical, modified, data, variant FROM Records WHERE variant != 2 ORDER BY modified DESC LIMIT ?1")?;

        let mut rows = retriever.query([limit])?;
        while let Some(row) = rows.next()? {
            let record_row = Record::borrow_from_row_unchecked(row);
            let rev_id = row.get_unwrap("rev");
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
    SELECT rev, parent_rev
    FROM Records
    WHERE rev IN (SELECT record_rev FROM Keys)

    UNION ALL

    SELECT r.rev, r.parent_rev
    FROM ancestors a
    INNER JOIN Records AS r ON a.parent_rev = r.rev
),
descendants AS (
    SELECT rev FROM ancestors

    UNION

    SELECT r.rev
    FROM Records AS r
    INNER JOIN descendants AS d ON r.parent_rev = d.rev
)
DELETE FROM Records WHERE rev NOT IN (SELECT rev FROM descendants);
",
            )?
            .execute([])?;
        Ok(())
    }

    pub fn touch_all(&self) -> rusqlite::Result<DateTime<Local>> {
        let now = Local::now();

        // create a new version for every non-leaf node, returning the required update pairs
        // for the Keys table
        let mut to_update: Vec<(i64, i64)> = Vec::new();

        let mut stmt = self.tx.prepare(
            "
INSERT INTO Records (canonical, data, modified, variant, parent_rev)
SELECT r.canonical, r.data, ?1, r.variant, r.rev
FROM Records AS r
WHERE r.rev IN (SELECT record_rev FROM Keys)
RETURNING rev, parent_rev",
        )?;

        for row in stmt.query_map([now], |row| {
            Ok((row.get_unwrap("rev"), row.get_unwrap("parent_rev")))
        })? {
            let (rev, parent_rev) = row?;
            to_update.push((rev, parent_rev));
        }

        for (rev, parent_rev) in to_update {
            self.tx
                .prepare_cached("UPDATE Keys SET record_rev = ?1 WHERE record_rev = ?2")?
                .execute((rev, parent_rev))?;
        }

        Ok(now)
    }

    /// Delete all inactive records.
    pub fn prune_all(&self) -> rusqlite::Result<()> {
        info!("Pruning all inactive revisions.");
        // delete everything which is not active. we don't need to set `parent_rev = NULL` because
        // of the `ON DELETE SET NULL` foreign key constraint
        self.tx
            .prepare("DELETE FROM Records WHERE rev NOT IN (SELECT record_rev FROM Keys)")?
            .execute([])?;
        Ok(())
    }

    /// Prune all 'oudated' entries: that is, those which are not a descendent of a currently
    /// active entry.
    pub fn prune_outdated(&self) -> rusqlite::Result<()> {
        info!("Pruning all outdated revisions.");
        self.tx
            .prepare(
                "
WITH RECURSIVE descendants AS (
  SELECT DISTINCT record_rev AS rev FROM Keys

  UNION ALL

  SELECT Records.rev
  FROM Records
  INNER JOIN descendants ON Records.parent_rev = descendants.rev
)
DELETE FROM Records WHERE rev NOT IN (SELECT rev FROM descendants)",
            )?
            .execute([])?;
        Ok(())
    }

    /// Prune all revisions which are not a descendent of a level `n` ancestor of an active
    /// revision.
    pub fn prune_outdated_keep(&self, retain: u32) -> rusqlite::Result<()> {
        info!("Pruning outdated revisions, retaining {retain} most recent revisisions.");
        self.tx
            .prepare(
                "
WITH RECURSIVE ancestors AS (
    SELECT rev, parent_rev, 0 as level
    FROM Records
    WHERE rev IN (SELECT record_rev FROM Keys)

    UNION ALL

    SELECT r.rev, r.parent_rev, a.level + 1
    FROM ancestors a
    INNER JOIN Records AS r ON a.parent_rev = r.rev
    WHERE a.level < ?1
),
descendants AS (
    SELECT rev FROM ancestors

    UNION

    SELECT r.rev
    FROM Records AS r
    INNER JOIN descendants AS d ON r.parent_rev = d.rev
)
DELETE FROM Records WHERE rev NOT IN (SELECT rev FROM descendants);
",
            )?
            .execute([retain])?;
        Ok(())
    }

    /// Check whether a specific revision is active.
    pub fn is_active(&self, rev_id: RevisionId) -> rusqlite::Result<bool> {
        self.tx
            .prepare("SELECT EXISTS (SELECT 1 FROM Keys WHERE record_rev = ?1)")?
            .query_one([rev_id.0], |row| row.get(0))
    }

    /// Delete inactive void records with exactly one child.
    pub fn prune_void(&self) -> rusqlite::Result<()> {
        info!("Pruning inactive void records.");
        self.tx
            .prepare(
                "
DELETE FROM Records
WHERE variant = 2
  AND rev NOT IN (SELECT record_rev FROM Keys)
  AND (SELECT count(*) FROM Records AS r WHERE r.parent_rev = Records.rev LIMIT 2) = 1",
            )?
            .execute([])?;
        Ok(())
    }

    /// Delete inactive deleted records which have no children.
    pub fn prune_deleted(&self) -> rusqlite::Result<()> {
        info!("Pruning deletion records with no children.");
        // the `parent_rev` is automatically set to null when the parent is deleted
        self.tx
            .prepare(
                "
DELETE FROM Records
WHERE variant = 1
  AND rev NOT IN (SELECT record_rev FROM Keys)
  AND NOT EXISTS (SELECT 1 FROM Records AS r WHERE r.parent_rev = Records.rev)",
            )?
            .execute([])?;
        self.prune_void()
    }

    /// Iterate over all active entries in the Records table, adding the revisions to the list
    /// which are later than the threshold date.
    pub fn rewind_all(&self, after: DateTime<Local>) -> rusqlite::Result<()> {
        let mut retriever = self
            .tx
            .prepare("SELECT canonical, rev FROM Records WHERE rev IN (SELECT record_rev FROM Keys) AND modified > ?1")?;

        let mut outdated: Vec<(String, i64)> = Vec::new();

        for rev in retriever.query_map([after], |row| {
            Ok((row.get_unwrap("canonical"), row.get_unwrap("rev")))
        })? {
            outdated.push(rev?);
        }

        for (canonical, row_id) in outdated {
            let new_row_id = create_rewind_target(&self.tx, &canonical, after)?;
            info!("Rewinding '{canonical}' from rev {row_id:0>4x} to rev {new_row_id:0>4x}");
            self.tx
                .prepare_cached("UPDATE Keys SET record_rev = ?1 WHERE record_rev = ?2")?
                .execute((new_row_id, row_id))?;
        }
        Ok(())
    }

    /// Iterate over all active entries in the Records table, adding the revisions to the list
    /// for which the provided closure returns true.
    pub fn filter_active_keys<F, T>(&self, mut f: F, buffer: &mut T) -> rusqlite::Result<()>
    where
        F: FnMut(Record<ArbitraryDataRef<'_>, &'_ str>) -> bool,
        T: Extend<RevisionId>,
    {
        let mut retriever = self
            .tx
            .prepare("SELECT rev, canonical, modified, data, variant FROM Records WHERE rev IN (SELECT record_rev FROM Keys)")?;

        let rows = retriever.query_map([], move |row| {
            let record_row = Record::borrow_from_row_unchecked(row);
            let rev_id: RevisionId = row.get_unwrap("rev");
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

    /// Iterate over all active canonical identifiers and apply the fallible closure `f` to each
    /// remote id.
    pub fn map_canonical_identifiers<E, F: FnMut(Identifier<&str>) -> Result<(), E>>(
        &self,
        deleted: bool,
        pattern: &str,
        mut f: F,
    ) -> Result<(), SnapshotMapErr<E>> {
        let mut selector = self.tx.prepare("SELECT canonical FROM Records WHERE rev IN (SELECT record_rev FROM Keys) AND variant = ?1  AND canonical GLOB ?2")?;
        let variant = if deleted { 1 } else { 0 };

        let mut rows = selector.query((variant, pattern))?;
        while let Some(row) = rows.next()? {
            if let ValueRef::Text(bytes) = row.get_ref_unwrap(0) {
                f(Identifier::from_string_unchecked(from_utf8(bytes).unwrap()))
                    .map_err(SnapshotMapErr::CallbackFailed)?;
            } else {
                panic!("Keys table has unexpected schema: column 'name' is not TEXT!");
            }
        }

        Ok(())
    }

    /// Iterate over all names in the Keys table and apply the fallible closure
    /// `f` to each key. If an error is returned by the closure, it is immediately propagated and
    /// the function exits early.
    pub fn map_identifiers<E, F: FnMut(&str) -> Result<(), E>>(
        &self,
        deleted: bool,
        pattern: &str,
        mut f: F,
    ) -> Result<(), SnapshotMapErr<E>> {
        let mut selector =
            self.tx.prepare("SELECT name FROM Keys INNER JOIN Records ON Keys.record_rev = Records.rev WHERE Records.variant = ?1 AND Keys.name GLOB ?2")?;
        let variant = if deleted { 1 } else { 0 };

        let mut rows = selector.query((variant, pattern))?;
        while let Some(row) = rows.next()? {
            if let ValueRef::Text(bytes) = row.get_ref_unwrap(0) {
                f(from_utf8(bytes).unwrap()).map_err(SnapshotMapErr::CallbackFailed)?;
            } else {
                panic!("Keys table has unexpected schema: column 'name' is not a TEXT!");
            }
        }

        Ok(())
    }

    pub fn equivalent_ids<I: AsKey, F>(&self, id: &I, mut f: F) -> rusqlite::Result<()>
    where
        F: FnMut(Identifier),
    {
        for id in self.tx.prepare("SELECT name FROM Keys WHERE record_rev = (SELECT record_rev FROM Keys WHERE name = ?1) AND instr(name, ':') != 0")?.query_map([id.as_key()], |row| {
            Ok(Identifier::from_string_unchecked(row.get(0)?))
        })? {
            f(id?);
        }

        Ok(())
    }
}
