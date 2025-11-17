use crate::{
    Config, RecordId, RemoteId,
    db::{
        RecordDatabase,
        state::{self, RecordIdState},
    },
    logger::{error, reraise, suggest},
};

/// Soft-delete the data associated with the provided citation key.
///
/// If record data exists for the provided key, the data is replaced with a 'deletion' marker, but not
/// removed from the database.
pub fn soft_delete<F: FnOnce() -> Vec<(regex::Regex, String)>>(
    citation_key: RecordId,
    replace: &Option<RemoteId>,
    record_db: &mut RecordDatabase,
    config: &Config<F>,
) -> Result<(), rusqlite::Error> {
    delete_impl(
        citation_key,
        record_db,
        config,
        |_, state| state.soft_delete(replace)?.commit(),
        |original_name, state| {
            error!("Key corresponds to record which is already deleted: '{original_name}'");
            state.commit()
        },
    )
}

/// Hard-delete the data associated with the provided citation key.
///
/// This deletes all data (including past data) as well as all keys in the `CitationKeys` table.
pub fn hard_delete<F: FnOnce() -> Vec<(regex::Regex, String)>>(
    citation_key: RecordId,
    record_db: &mut RecordDatabase,
    config: &Config<F>,
) -> Result<(), rusqlite::Error> {
    delete_impl(
        citation_key,
        record_db,
        config,
        |_, state| state.hard_delete()?.commit(),
        |_, state| state.hard_delete()?.commit(),
    )
}

/// Handle the cases where the key is not in the database and defer deletion to the callback.
fn delete_impl<F, R, D>(
    citation_key: RecordId,
    record_db: &mut RecordDatabase,
    config: &Config<F>,
    entry_callback: R,
    deleted_callback: D,
) -> Result<(), rusqlite::Error>
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    R: FnOnce(String, state::State<'_, state::EntryRecordKey>) -> Result<(), rusqlite::Error>,
    D: FnOnce(String, state::State<'_, state::DeletedRecordKey>) -> Result<(), rusqlite::Error>,
{
    match record_db.state_from_record_id(citation_key, &config.alias_transform)? {
        RecordIdState::Entry(original_name, _, state) => entry_callback(original_name, state)?,
        RecordIdState::Deleted(original_name, _, state) => deleted_callback(original_name, state)?,
        RecordIdState::NullRemoteId(mapped_key, state) => {
            state.commit()?;
            error!("Cannot delete null record data: {mapped_key}");
            suggest!("Delete null records using `autobib util evict`.");
        }
        RecordIdState::Unknown(unknown) => {
            let maybe_normalized = unknown.combine_and_commit()?;
            error!("Cannot delete key not in database: {maybe_normalized}");
        }
        RecordIdState::UndefinedAlias(alias) => {
            error!("Cannot delete undefined alias: {alias}");
        }
        RecordIdState::InvalidRemoteId(record_error) => {
            reraise(&record_error);
        }
    };
    Ok(())
}
