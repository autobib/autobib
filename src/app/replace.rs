use anyhow::bail;

use crate::{
    app::cli::OnConflict,
    config::Config,
    db::{
        Tx,
        state::{
            DatabaseResponse, DisambiguatedRecordState, IsEntry, State, replace_hard_unchecked,
        },
    },
    entry::{MutableEntryData, RawEntryData},
    logger::{suggest, warn},
    path_hash::{AttachmentRenameOutcome, AttachmentRoot},
    record::{Key, KeyedRecord},
};

/// The closure `data_cb` is a function which accepts a transaction and the entry data for the
/// record to be replaced and returns a record corresponding to its replacement.
#[expect(clippy::too_many_arguments)]
pub fn replace<'conn, G>(
    identifier: Key,
    tx: Tx<'conn>,
    cfg: &Config,
    data_cb: G,
    hard: bool,
    update_aliases: bool,
    on_conflict: OnConflict,
    root: Option<AttachmentRoot<false>>,
) -> Result<(), anyhow::Error>
where
    G: FnOnce(
        Tx<'conn>,
        &RawEntryData,
    ) -> anyhow::Result<(KeyedRecord<RawEntryData>, State<'conn, IsEntry>)>,
{
    // first, get the data for the identifier that will be replaced
    let (original_record, (tx, original_row_id)) = match DatabaseResponse::determine(
        tx,
        identifier,
        &cfg.alias_transform,
    )?
    .require_record()?
    {
        Some((_, DisambiguatedRecordState::Entry(record_row, state))) => {
            (record_row, state.into_parts())
        }
        Some((_, DisambiguatedRecordState::Deleted(record_row, state))) => {
            state.commit()?;
            bail!(
                "Cannot replace deleted record with canonical id '{}'",
                record_row.canonical
            );
        }
        Some((_, DisambiguatedRecordState::Void(record_row, state))) => {
            state.commit()?;
            bail!(
                "Cannot replace voided record with canonical id '{}'",
                record_row.canonical
            );
        }
        // `set_failed` was already called here
        None => return Ok(()),
    };

    // next, get the target data using the callback
    let (replacement_record, replacement_row) = data_cb(tx, &original_record.data)?;

    // make sure they aren't the same row
    if replacement_record.record.canonical == original_record.canonical {
        bail!(
            "replacement identifier '{}' is equivalent to the current identifier",
            replacement_record.record.canonical
        );
    }

    // update the target row data
    let mut incoming_record = MutableEntryData::from_entry_data(&replacement_record.record.data);
    crate::app::edit::merge_record_data(
        on_conflict,
        &mut incoming_record,
        Some(&original_record.data),
        &original_record.canonical,
    )?;
    let replacement_row =
        replacement_row.modify(&RawEntryData::from_entry_data(&incoming_record))?;

    let (tx, replacement_row_id) = replacement_row.state.into_parts();

    // FIXME: find a way to hold 'joint state' in some reasonable way
    if hard {
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
            .delete_soft(Some(&replacement_record.record.canonical), update_aliases)?
            .commit()?;
    }

    // try to migrate attachments, or warn on orphaned
    let Some(root) = root else {
        return Ok(());
    };

    match root.rename(
        &original_record.canonical,
        &replacement_record.record.canonical,
    )? {
        AttachmentRenameOutcome::ToExists(source, target) => {
            warn!(
                "Could not merge attachment directories:\n  original: {}\n  replacement: {}",
                source.display(),
                target.display()
            );
            suggest!(
                "Move attachment files from the original directory to the replacement directory"
            );
            Ok(())
        }
        AttachmentRenameOutcome::FromMissing | AttachmentRenameOutcome::Ok => Ok(()),
    }
}
