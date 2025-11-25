use std::iter::once;

use anyhow::bail;

use crate::{
    RemoteId,
    app::{cli::OnConflict, merge_record_data},
    db::state::{RecordIdState, RecordRow},
    entry::{MutableEntryData, RawEntryData},
    http::Client,
    logger::{error, suggest},
    normalize::{Normalization, Normalize},
    record::{RecursiveRemoteResponse, get_remote_response_recursive},
};

/// Update the record id corresponding to the [`RecordIdState`] using data returned by
/// `data_callback`.
///
/// If the record exists, update it either with the provided data, or remote data if none.
///
/// If the record is null, it cannot be updated by provided data, and will only update if there is
/// new data to retrieve from remote.
pub fn update<C: Client>(
    on_conflict: OnConflict,
    record_id_state: RecordIdState,
    provided_data: Option<MutableEntryData>,
    client: &C,
    normalization: &Normalization,
    revive: bool,
) -> Result<(), anyhow::Error> {
    match record_id_state {
        RecordIdState::Entry(
            citation_key,
            RecordRow {
                data, canonical, ..
            },
            state,
        ) => {
            if revive {
                state.commit()?;
                bail!("Record already exists");
            } else {
                let mut new_raw_data = if let Some(data) = provided_data {
                    data
                } else if canonical.is_local() {
                    state.commit()?;
                    bail!(
                        "Cannot update local record using remote data: use `autobib edit` or the `--from-bibtex` or `--from-key` options."
                    );
                } else {
                    data_from_remote(canonical, client)?.0
                };

                new_raw_data.normalize(normalization);

                let mut existing_record = MutableEntryData::from_entry_data(&data);
                merge_record_data(
                    on_conflict,
                    &mut existing_record,
                    once(&new_raw_data),
                    &citation_key,
                )?;

                state
                    .modify(&RawEntryData::from_entry_data(&existing_record))?
                    .commit()?;
            }
        }
        RecordIdState::Deleted(citation_key, data, state) => {
            if revive {
                let mut raw_data = if data.canonical.is_local() {
                    state.commit()?;
                    bail!(
                        "Cannot update local record using remote data: use `autobib edit` or the `--from-bibtex` or `--from-key` options."
                    );
                } else {
                    data_from_remote(data.canonical, client)?.0
                };

                raw_data.normalize(normalization);
                state
                    .reinsert(&RawEntryData::from_entry_data(&raw_data))?
                    .commit()?;
            } else {
                state.commit()?;
                error!("Cannot update soft-deleted row '{citation_key}'.");
                suggest!("Undo first, or use `autobib update --revive` to insert new data.");
            }
        }
        RecordIdState::Void(key, data, void) => {
            void.commit()?;
            error!("Record exists but has been voided: {key}");
            if data.canonical.is_local() {
                suggest!(
                    "Use `autobib local` to insert new data, or find an existing version using `autobib log --all`."
                );
            } else {
                suggest!(
                    "Use `autobib get` to get new data, or find an existing version using `autobib log --all`."
                );
                suggest!("Use `autobib hist revive` to insert new data.");
            }
        }
        RecordIdState::NullRemoteId(mapped_remote_id, null_row) => {
            if provided_data.is_some() {
                null_row.commit()?;
                // cannot update a null record with provided data
                bail!("Null record can only be updated with remote data");
            } else {
                // do not need to check is_local since local ids cannot be in the null records
                // table
                let (mut data, canonical) = data_from_remote(mapped_remote_id.mapped, client)?;
                data.normalize(normalization);
                null_row
                    .delete()?
                    .insert_entry_data(&data, &canonical)?
                    .commit()?;
            };
        }
        RecordIdState::Unknown(unknown) => {
            let maybe_normalized = unknown.combine_and_commit()?;
            error!("Record does not exist in database: {maybe_normalized}");
            if !maybe_normalized.mapped.is_local() {
                suggest!("Use `autobib get` to retrieve record");
            }
        }
        RecordIdState::UndefinedAlias(alias) => {
            bail!("Undefined alias: '{alias}'");
        }
        RecordIdState::InvalidRemoteId(err) => bail!("{err}"),
    };
    Ok(())
}

fn data_from_remote<C: Client>(
    remote_id: RemoteId,
    client: &C,
) -> Result<(MutableEntryData, RemoteId), anyhow::Error> {
    match get_remote_response_recursive(remote_id, client)? {
        RecursiveRemoteResponse::Exists(record_data, canonical) => Ok((record_data, canonical)),
        RecursiveRemoteResponse::Null(null_remote_id) => {
            bail!("Remote data for canonical id '{null_remote_id}' is null");
        }
    }
}
