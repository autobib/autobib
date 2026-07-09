use crate::{
    Config, RecordId, RemoteId,
    db::{
        RecordDatabase,
        state::{self, DatabaseResponse},
    },
    logger::{error, reraise, suggest},
    record::KeyedRecord,
};

/// Soft-delete the data associated with the provided identifier.
///
/// If record data exists for the provided key, the data is replaced with a 'deletion' marker, but not
/// removed from the database.
pub fn soft_delete(
    id: RecordId,
    replace: &Option<RemoteId>,
    record_db: &mut RecordDatabase,
    config: &Config,
    update_aliases: bool,
) -> Result<Option<RemoteId>, rusqlite::Error> {
    delete_impl(
        id,
        record_db,
        config,
        |_, state| {
            state
                .delete_soft(replace.as_ref(), update_aliases)?
                .commit()
        },
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
/// This deletes all data (including past data) as well as all identifiers in the `Identifiers` table.
pub fn hard_delete(
    id: RecordId,
    record_db: &mut RecordDatabase,
    config: &Config,
) -> Result<Option<RemoteId>, rusqlite::Error> {
    delete_impl(
        id,
        record_db,
        config,
        |_, state| state.delete_hard()?.commit(),
        |_, state| state.delete_hard()?.commit(),
        |_, state| state.delete_hard()?.commit(),
    )
}

/// Handle the cases where the key is not in the database and defer deletion to the callback.
fn delete_impl<R, D, V>(
    id: RecordId,
    record_db: &mut RecordDatabase,
    config: &Config,
    entry_callback: R,
    deleted_callback: D,
    voided_callback: V,
) -> Result<Option<RemoteId>, rusqlite::Error>
where
    R: FnOnce(String, state::State<'_, state::IsEntry>) -> Result<(), rusqlite::Error>,
    D: FnOnce(String, state::State<'_, state::IsDeleted>) -> Result<(), rusqlite::Error>,
    V: FnOnce(String, state::State<'_, state::IsVoid>) -> Result<(), rusqlite::Error>,
{
    match record_db.state_from_record_id(id, &config.alias_transform)? {
        DatabaseResponse::Entry(record, state) => {
            entry_callback(record.key, state)?;
            return Ok(Some(record.record.canonical));
        }
        DatabaseResponse::Deleted(record, state) => {
            deleted_callback(record.key, state)?;
            return Ok(Some(record.record.canonical));
        }
        DatabaseResponse::Void(
            KeyedRecord {
                key: original_name, ..
            },
            state,
        ) => {
            voided_callback(original_name, state)?;
        }
        DatabaseResponse::NullRemoteId(mapped_key, state) => {
            state.commit()?;
            error!("Cannot delete null record data: {mapped_key}");
            suggest!("Delete null records using `autobib clean database --evict`.");
        }
        DatabaseResponse::Unknown(unknown) => {
            let maybe_normalized = unknown.combine_and_commit()?;
            error!("Cannot delete key not in database: {maybe_normalized}");
        }
        DatabaseResponse::UndefinedAlias(alias) => {
            error!("Cannot delete undefined alias: {alias}");
        }
        DatabaseResponse::InvalidRemoteId(record_error) => {
            reraise(&record_error);
        }
    };
    Ok(None)
}
