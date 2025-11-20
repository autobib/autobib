use std::iter::once;

use anyhow::bail;

use crate::{
    RemoteId,
    app::{cli::OnConflict, merge_record_data},
    db::state::{EntryRowData, RecordIdState},
    entry::{MutableEntryData, RawEntryData},
    http::Client,
    logger::{error, suggest},
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
) -> Result<(), anyhow::Error> {
    match record_id_state {
        RecordIdState::Entry(
            citation_key,
            EntryRowData {
                data, canonical, ..
            },
            state,
        ) => {
            let new_raw_data = if let Some(data) = provided_data {
                data
            } else if canonical.is_local() {
                bail!(
                    "Cannot update local record using remote data: use `autobib edit` or the `--from-bibtex` or `--from-key` options."
                );
            } else {
                data_from_remote(canonical, client)?.0
            };

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
        RecordIdState::Deleted(citation_key, _, state) => {
            state.commit()?;
            bail!("Cannot update soft-deleted row '{citation_key}'.");
        }
        RecordIdState::NullRemoteId(mapped_remote_id, null_row) => {
            if provided_data.is_some() {
                // cannot update a null record with provided data
                bail!("Null record can only be updated with remote data");
            } else {
                // do not need to check is_local since local ids cannot be in the null records
                // table
                let (data, canonical) = data_from_remote(mapped_remote_id.mapped, client)?;
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
