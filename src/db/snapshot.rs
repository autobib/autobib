use chrono::{DateTime, Local, TimeDelta};

use super::{Tx, state::RevisionId};
use crate::{
    db::{AsKey, state::create_rewind_target},
    logger::info,
    record::{Alias, LegacyAlias},
};

pub struct Snapshot<'conn> {
    pub(super) tx: Tx<'conn>,
}

impl<'conn> Snapshot<'conn> {
    /// Commit the changes made in this snapshot.
    pub fn commit(self) -> rusqlite::Result<()> {
        self.tx.commit()
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

    /// Rename an alias, returning the status of the renaming.
    pub fn rename_alias(
        &mut self,
        old: &LegacyAlias,
        new: &Alias,
    ) -> Result<RenameAliasResult, rusqlite::Error> {
        let mut updater = self
            .tx
            .prepare("UPDATE Keys SET name = ?1 WHERE name = ?2")?;
        match flatten_constraint_violation(updater.execute((new.as_key(), old.as_ref())))? {
            Constraint::Satisfied(0) => Ok(RenameAliasResult::SourceMissing),
            Constraint::Satisfied(_) => Ok(RenameAliasResult::Renamed),
            Constraint::Violated => Ok(RenameAliasResult::TargetExists),
        }
    }

    /// Delete an alias, returning the status of the deletion.
    ///
    /// This method returns `true` if the alias was deleted, and `false` otherwise.
    pub fn delete_alias(&mut self, alias: &LegacyAlias) -> Result<bool, rusqlite::Error> {
        let mut deleter = self.tx.prepare("DELETE FROM Keys WHERE name = ?1")?;
        Ok(deleter.execute((alias.as_ref(),))? != 0)
    }

    /// Delete all rows from `NullRecords`.
    pub fn evict_cache(&mut self) -> Result<(), rusqlite::Error> {
        let num_deleted = self.tx.prepare("DELETE FROM NullRecords")?.execute(())?;
        info!("Removed {num_deleted} cached null records.");
        Ok(())
    }

    /// Delete all rows from `NullRecords` which are at least a given age (in seconds)
    pub fn evict_cache_max_age(&mut self, seconds: u32) -> Result<(), rusqlite::Error> {
        let threshold = Local::now() - TimeDelta::seconds(seconds.into());
        let num_deleted = self
            .tx
            .prepare("DELETE FROM NullRecords WHERE attempted <= ?1")?
            .execute((threshold,))?;
        info!("Removed {num_deleted} cached null records.");
        Ok(())
    }
}

/// Take the result of a SQLite operation and extract a constraint violation.
pub fn flatten_constraint_violation<T>(
    res: Result<T, rusqlite::Error>,
) -> Result<Constraint<T>, rusqlite::Error> {
    match res {
        Ok(t) => Ok(Constraint::Satisfied(t)),
        Err(err) => match err.sqlite_error_code() {
            Some(rusqlite::ErrorCode::ConstraintViolation) => Ok(Constraint::Violated),
            _ => Err(err),
        },
    }
}

/// The outcome of flattening a constraint violation error.
pub enum Constraint<T> {
    /// All constraints were satisfied during the database operation; result of the operation.
    Satisfied(T),
    /// A constraint was not satisfied.
    Violated,
}

/// The result of renaming an alias.
#[must_use]
pub enum RenameAliasResult {
    /// The alias was successfully renamed.
    Renamed,
    /// The new alias name already exists.
    TargetExists,
    /// The existing alias name did not exist.
    SourceMissing,
}
