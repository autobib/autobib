use crate::{
    Config, RecordId, RemoteId,
    db::{
        RecordDatabase,
        state::{self, RecordIdState},
    },
    logger::{error, reraise, suggest},
};

/// Soft-delete the data associated with the provided identifier.
///
/// If record data exists for the provided key, the data is replaced with a 'deletion' marker, but not
/// removed from the database.
pub fn soft_delete<F: FnOnce() -> Vec<(regex::Regex, String)>>(
    id: RecordId,
    replace: &Option<RemoteId>,
    record_db: &mut RecordDatabase,
    config: &Config<F>,
    update_aliases: bool,
) -> Result<(), rusqlite::Error> {
    delete_impl(
        id,
        record_db,
        config,
        |_, state| state.soft_delete(replace, update_aliases)?.commit(),
        |original_name, state| {
            error!("Key corresponds to record which is already deleted: '{original_name}'");
            state.commit()
        },
        |original_name, state| {
            error!("Key corresponds to voided record: '{original_name}'");
            state.commit()
        },
    )
}

/// Hard-delete the data associated with the provided identifier.
///
/// This deletes all data (including past data) as well as all keys in the `Identifiers` table.
pub fn hard_delete<F: FnOnce() -> Vec<(regex::Regex, String)>>(
    id: RecordId,
    record_db: &mut RecordDatabase,
    config: &Config<F>,
) -> Result<(), rusqlite::Error> {
    delete_impl(
        id,
        record_db,
        config,
        |_, state| state.hard_delete()?.commit(),
        |_, state| state.hard_delete()?.commit(),
        |_, state| state.hard_delete()?.commit(),
    )
}

/// Handle the cases where the key is not in the database and defer deletion to the callback.
fn delete_impl<F, R, D, V>(
    id: RecordId,
    record_db: &mut RecordDatabase,
    config: &Config<F>,
    entry_callback: R,
    deleted_callback: D,
    voided_callback: V,
) -> Result<(), rusqlite::Error>
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    R: FnOnce(String, state::State<'_, state::IsEntry>) -> Result<(), rusqlite::Error>,
    D: FnOnce(String, state::State<'_, state::IsDeleted>) -> Result<(), rusqlite::Error>,
    V: FnOnce(String, state::State<'_, state::IsVoid>) -> Result<(), rusqlite::Error>,
{
    match record_db.state_from_record_id(id, &config.alias_transform)? {
        RecordIdState::Entry(original_name, _, state) => entry_callback(original_name, state)?,
        RecordIdState::Deleted(original_name, _, state) => deleted_callback(original_name, state)?,
        RecordIdState::Void(original_name, _, state) => voided_callback(original_name, state)?,
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
