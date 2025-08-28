use rusqlite::Connection;

use crate::{
    db::application_id,
    error::DatabaseError,
    logger::{debug, info},
};

pub fn migrate(conn: &mut Connection, v: i32) -> Result<(), DatabaseError> {
    info!("Migrating database from v{v} to v{}", v + 1);
    match v {
        0 => {
            // since we did not use the application id in v0, check the table schemas to
            // make sure everything is in order
            let tx = conn.transaction()?;

            debug!("Checking schema for original tables.");
            check_table_schema(&tx, "Records", include_str!("migrate/v0/records.sql"))?;
            check_table_schema(
                &tx,
                "CitationKeys",
                include_str!("migrate/v0/citation_keys.sql"),
            )?;
            check_table_schema(
                &tx,
                "NullRecords",
                include_str!("migrate/v0/null_records.sql"),
            )?;
            check_table_schema(&tx, "Changelog", include_str!("migrate/v0/changelog.sql"))?;
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
        // this is only reachable if the user_version was set by a different program
        _ => return Err(DatabaseError::InvalidDatabase),
    }

    // after migration is finished, update the user version to reflect this
    conn.pragma_update(None, "user_version", v + 1)?;
    Ok(())
}

/// Validate the schema of an existing table, or return an appropriate error.
fn check_table_schema(
    tx: &rusqlite::Transaction,
    table_name: &str,
    expected_schema: &str,
) -> Result<(), DatabaseError> {
    let mut table_selector = tx.prepare(include_str!("migrate/v0/get_table_schema.sql"))?;
    let mut record_rows = table_selector.query([table_name])?;
    match record_rows.next() {
        Ok(Some(row)) => {
            let table_schema: String = row.get("sql")?;
            if table_schema == expected_schema {
                Ok(())
            } else {
                Err(DatabaseError::Migration(
                    0,
                    format!("Table '{table_name}' has invalid schema:\n{table_schema}",),
                ))
            }
        }
        Ok(None) => Err(DatabaseError::Migration(
            0,
            format!("Missing table '{table_name}'"),
        )),
        Err(why) => Err(why.into()),
    }
}
