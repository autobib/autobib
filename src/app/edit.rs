use std::str::FromStr;

use anyhow::Result;
use serde_bibtex::token::EntryKey;

use super::UpdateMode;

use crate::{
    db::{
        state::{RecordRow, State},
        RawRecordData, RecordData,
    },
    entry::Entry,
    error::MergeError,
    logger::{error, info, warn},
    record::{Alias, Record},
    term::{Confirm, Editor, EditorConfig},
};

/// Edit a record and update the entry corresponding to the [`RecordRow`].
pub fn edit_record_and_update(
    row: &State<RecordRow>,
    record: Record,
) -> Result<Entry<RawRecordData>, anyhow::Error> {
    let Record {
        key,
        data,
        canonical,
    } = record;

    let mut entry = Entry::new(EntryKey::new(key).map_err(|res| res.error)?, data);

    let editor = Editor::new(EditorConfig { suffix: ".bib" });

    if let Some(new_entry) = editor.edit(&entry)? {
        let Entry {
            key: ref new_key,
            record_data: ref new_record_data,
        } = new_entry;

        if new_key != entry.key() {
            let alias = Alias::from_str(new_key.as_ref())?;
            info!("Creating new alias '{alias}' for '{canonical}'");
            row.add_alias(&alias)?;
        }

        if new_record_data != entry.data() {
            info!("Updating cached data for '{canonical}'");
            row.save_to_changelog()?;
            row.update_row_data(new_record_data)?;
        }

        entry = new_entry;
    }

    Ok(entry)
}

/// Merge an iterator of [`RawRecordData`] into existing data, using the merge rules as specified
/// by the passed [`UpdateMode`].
pub fn merge_record_data<'a>(
    mode: UpdateMode,
    existing_record: &mut RecordData,
    new_raw_data: impl Iterator<Item = &'a RawRecordData>,
    citation_key: impl std::fmt::Display,
) -> Result<(), MergeError> {
    match mode {
        UpdateMode::PreferCurrent => {
            info!("Updating {citation_key} with new data, skipping existing fields");
            for data in new_raw_data {
                existing_record.merge_or_skip(data)?;
            }
        }
        UpdateMode::PreferIncoming => {
            info!("Updating {citation_key} with new data, overwriting existing fields");
            for data in new_raw_data {
                existing_record.merge_or_overwrite(data)?;
            }
        }
        UpdateMode::Prompt => {
            info!("Updating {citation_key} with new data, prompting on conflict");
            for data in new_raw_data {
                existing_record.merge_with_callback(data, |key, current, incoming| {
                    eprintln!("Conflict for the field '{key}':");
                    eprintln!("   Current value: {current}");
                    eprintln!("  Incoming value: {incoming}");
                    let prompt = Confirm::new("Accept incoming value?", false);
                    match prompt.confirm() {
                        Ok(true) => incoming.to_owned(),
                        Ok(false) => current.to_owned(),
                        Err(error) => {
                            error!("{error}");
                            warn!("Keeping current value for '{key}'");
                            current.to_owned()
                        }
                    }
                })?;
            }
        }
    }
    Ok(())
}
