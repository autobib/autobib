use rusqlite::Connection;

use crate::{
    db::{application_id, validate::check_table_schema},
    error::DatabaseError,
    logger::{debug, warn},
};

pub fn migrate(conn: &mut Connection, v: i32) -> Result<(), DatabaseError> {
    warn!("Migrating database from v{v} to v{}", v + 1);
    match v {
        0 => {
            // since we did not use the application id in v0, check the table schemas to
            // make sure everything is in order
            let tx = conn.transaction()?.into();

            for (tbl_name, schema) in [
                ("Records", include_str!("migrate/v0/records.sql")),
                ("CitationKeys", include_str!("migrate/v0/citation_keys.sql")),
                ("NullRecords", include_str!("migrate/v0/null_records.sql")),
                ("Changelog", include_str!("migrate/v0/changelog.sql")),
            ] {
                debug!("Checking schema for table '{tbl_name}'.");
                if let Some(err) = check_table_schema(&tx, tbl_name, schema)? {
                    return Err(DatabaseError::Migration(0, err.to_string()));
                }
            }
            tx.commit()?;

            // procedure from:
            // https://www.sqlite.org/lang_altertable.html#making_other_kinds_of_table_schema_changes
            debug!("Turning off foreign key checks");
            conn.pragma_update(None, "foreign_keys", "OFF")?;
            let tx = conn.transaction()?;

            debug!("Creating new temporary table `tmp_new_CitationKeys`");
            tx.execute(include_str!("migrate/v0/make_tmp_table.sql"), ())?;

            debug!("Copying keys into the temporary table");
            tx.execute(include_str!("migrate/v0/copy_keys.sql"), ())?;

            debug!("Dropping the original `CitationKeys` table");
            tx.execute(include_str!("migrate/v0/drop_original.sql"), ())?;

            debug!("Renaming the temporary table to `CitationKeys`");
            tx.execute(include_str!("migrate/v0/rename_tmp_table.sql"), ())?;

            tx.pragma_update(None, "writable_schema", "ON")?;
            // this is a bit horrifying since we are manually updating the schema text; but since
            // the parsed version of the schema text is unchanged it is safe
            // this ensures that the schema text matches the exact schema text in v1 after
            // migration
            tx.execute(
                "UPDATE sqlite_schema SET sql=?1 WHERE type='table' AND name='CitationKeys'",
                (include_str!("migrate/v0/citation_keys_new.sql"),),
            )?;

            tx.pragma_update(None, "writable_schema", "OFF")?;

            debug!("Checking foreign key constraints in new table");
            let mut num_faults: usize = 0;
            tx.pragma_query(None, "foreign_key_check", |_| {
                num_faults += 1;
                Ok(())
            })?;
            if num_faults != 0 {
                return Err(DatabaseError::Migration(
                    0,
                    format!(
                        "Failed to update `CitationKeys` table: foreign key check returned {num_faults} errors"
                    ),
                ));
            }

            tx.commit()?;

            debug!("Successfully migrated tables. Re-enabling foreign key checks.");
            conn.pragma_update(None, "foreign_keys", "ON")?;

            debug!("Setting the application id.");
            conn.pragma_update(None, "application_id", application_id())?;
        }
        1 => return Err(DatabaseError::CannotMigrate(1)),
        // this is only reachable if the user_version was set by a different program
        _ => return Err(DatabaseError::InvalidDatabase),
    }

    // after migration is finished, update the user version to reflect this
    conn.pragma_update(None, "user_version", v + 1)?;
    Ok(())
}
