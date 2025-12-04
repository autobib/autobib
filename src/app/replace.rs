use anyhow::bail;

use crate::{
    app::cli::OnConflict,
    db::{
        Tx,
        state::{DisambiguatedRecordRow, IsEntry, RecordIdState, State, replace_hard_unchecked},
    },
    entry::{MutableEntryData, RawEntryData},
    logger::warn,
    record::{Record, RecordId},
};

pub fn replace<'conn, F, G>(
    identifier: RecordId,
    tx: Tx<'conn>,
    cfg: &crate::config::Config<F>,
    data_cb: G,
    hard: bool,
    update_aliases: bool,
    on_conflict: OnConflict,
) -> Result<(), anyhow::Error>
where
    F: FnOnce() -> Vec<(regex::Regex, String)>,
    G: FnOnce(
        Tx<'conn>,
        &RawEntryData,
    ) -> anyhow::Result<(Record<RawEntryData>, State<'conn, IsEntry>)>,
{
    // first, get the data for the identifier that will be replaced
    let (original_record, (tx, original_row_id)) =
        match RecordIdState::determine(tx, identifier, &cfg.alias_transform)?.require_record()? {
            Some((_, DisambiguatedRecordRow::Entry(record_row, state))) => {
                (record_row, state.into_parts())
            }
            Some((_, DisambiguatedRecordRow::Deleted(record_row, state))) => {
                state.commit()?;
                bail!(
                    "Cannot replace deleted record with canonical id '{}'",
                    record_row.canonical
                );
            }
            Some((_, DisambiguatedRecordRow::Void(record_row, state))) => {
                state.commit()?;
                bail!(
                    "Cannot replace voided record with canonical id '{}'",
                    record_row.canonical
                );
            }
            // `set_failed` was already called here
            None => return Ok(()),
        };

    // next, get the target data. maybe it doesn't exist in the database yet, so it
    // has to be retrieved
    let (replacement_record, replacement_row) = data_cb(tx, &original_record.data)?;

    // make sure they aren't the same row
    if replacement_record.canonical == original_record.canonical {
        bail!(
            "replacement identifier '{}' is equivalent to the current identifier",
            replacement_record.canonical
        );
    }

    // update the target row data if requested
    let mut existing_record = MutableEntryData::from_entry_data(&original_record.data);
    crate::app::edit::merge_record_data(
        on_conflict,
        &mut existing_record,
        Some(&replacement_record.data).into_iter(),
        &replacement_record.canonical,
    )?;
    let replacement_row =
        replacement_row.modify(&RawEntryData::from_entry_data(&existing_record))?;

    let (tx, replacement_row_id) = replacement_row.into_parts();

    // FIXME: find a way to hold 'joint state' in some reasonable way
    if hard {
        if update_aliases {
            warn!("Redundant flag `--update-aliases` is implied by `--hard`");
        }

        replace_hard_unchecked(
            tx,
            original_row_id,
            &original_record.canonical,
            replacement_row_id,
        )?
        .commit()?;
    } else {
        let original_row = State::init_unchecked(tx, original_row_id);
        original_row
            .delete_soft(Some(&replacement_record.canonical), update_aliases)?
            .commit()?;
    }

    Ok(())
}
