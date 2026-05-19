use std::{fs::read_to_string, iter::once, path::Path, str::FromStr};

use anyhow::bail;

use crate::{
    Config,
    app::{cli::OnConflict, merge_record_data},
    db::{
        Tx,
        state::{ArbitraryData, RecordIdState, RecordRow},
    },
    entry::{Entry, MutableEntryData, RawEntryData},
    http::Client,
    logger::{error, suggest},
    normalize::{Normalization, Normalize},
    record::{RecordId, RemoteId},
    record::{RecursiveRemoteResponse, get_remote_response_recursive},
};

/// Update the record id corresponding to the [`RecordIdState`] using data returned by
/// `data_callback`.
///
/// If the record exists, update it either with the provided data, or remote data if none.
///
/// If the record is null, it cannot be updated by provided data, and will only update if there is
/// new data to retrieve from remote.
pub fn update<F>(
    on_conflict: OnConflict,
    record_id_state: RecordIdState,
    provided_data: Option<MutableEntryData>,
    normalization: &Normalization,
    revive: bool,
    produce_data: F,
) -> Result<(), anyhow::Error>
where
    F: FnOnce(RemoteId) -> Result<MutableEntryData, anyhow::Error>,
{
    match record_id_state {
        RecordIdState::Entry(
            id,
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
                } else {
                    match produce_data(canonical) {
                        Ok(data) => data,
                        Err(e) => {
                            state.commit()?;
                            return Err(e);
                        }
                    }
                };

                new_raw_data.normalize(normalization);

                let mut existing_record = MutableEntryData::from_entry_data(&data);
                merge_record_data(on_conflict, &mut existing_record, once(&new_raw_data), &id)?;

                state
                    .modify(&RawEntryData::from_entry_data(&existing_record))?
                    .commit()?;
            }
        }
        RecordIdState::Deleted(id, data, state) => {
            if revive {
                let mut raw_data = if let Some(data) = provided_data {
                    data
                } else {
                    match produce_data(data.canonical) {
                        Ok(data) => data,
                        Err(e) => {
                            state.commit()?;
                            return Err(e);
                        }
                    }
                };

                raw_data.normalize(normalization);
                state
                    .reinsert(&RawEntryData::from_entry_data(&raw_data))?
                    .commit()?;
            } else {
                state.commit()?;
                error!("Cannot update soft-deleted row '{id}'.");
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
            null_row.commit()?;
            bail!("Cannot update null record with identifier: {mapped_remote_id}");
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

pub fn data_from_remote<C: Client>(
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

pub fn data_from_key<'conn, F: FnOnce() -> Vec<(regex::Regex, String)>>(
    tx: Tx<'conn>,
    record_id: RecordId,
    cfg: &Config<F>,
) -> Result<(MutableEntryData, Tx<'conn>), anyhow::Error> {
    match RecordIdState::determine(tx, record_id, &cfg.alias_transform)? {
        RecordIdState::Entry(_, entry_row_data, state) => Ok((
            MutableEntryData::from_entry_data(&entry_row_data.data),
            state.into_tx(),
        )),
        RecordIdState::Deleted(_, _, state) => {
            state.commit()?;
            bail!("Cannot read update data from deleted row");
        }
        RecordIdState::Void(_, _, state) => {
            state.commit()?;
            bail!("Cannot read update data from voided row");
        }
        RecordIdState::NullRemoteId(_, state) => {
            state.commit()?;
            bail!("Cannot read update data from null record");
        }
        RecordIdState::Unknown(unknown) => {
            unknown.combine_and_commit()?;
            bail!("Cannot read update data from record not present in database");
        }
        RecordIdState::UndefinedAlias(_) => {
            bail!("Cannot read update data from undefined alias");
        }
        RecordIdState::InvalidRemoteId(record_error) => {
            bail!("Cannot read update data: {record_error}");
        }
    }
}

pub fn data_from_rev(
    tx: &Tx<'_>,
    rev: crate::db::state::RevisionId,
) -> Result<MutableEntryData, anyhow::Error> {
    let Some(row) = RecordRow::load(tx, rev)? else {
        bail!("Revision '{rev}' does not exist in the database!");
    };

    match row.data {
        ArbitraryData::Entry(raw_entry_data) => {
            Ok(MutableEntryData::from_entry_data(&raw_entry_data))
        }
        ArbitraryData::Deleted(_) => bail!("Cannot read update data from deleted row"),
        ArbitraryData::Void => bail!("Cannot read update data from voided row"),
    }
}

/// Obtain data from a bibtex record at a provided path.
pub fn data_from_path<P: AsRef<Path>>(path: P) -> Result<MutableEntryData, anyhow::Error> {
    let bibtex = read_to_string(path)?;
    let entry = Entry::<MutableEntryData>::from_str(&bibtex)?;
    Ok(entry.record_data)
}
