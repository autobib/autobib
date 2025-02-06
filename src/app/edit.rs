use std::str::FromStr;

use anyhow::Result;

use super::UpdateMode;

use crate::{
    db::state::{RecordRow, State},
    entry::{ConflictResolved, Entry, EntryData, RecordData},
    error::MergeError,
    logger::{error, info, set_failed, suggest, warn},
    record::Alias,
    term::{Editor, EditorConfig, Input},
};

/// Edit a record and update the entry corresponding to the [`RecordRow`]. Returns the edited
/// record, saving the data.
pub fn edit_record_and_update(
    row: &State<RecordRow>,
    mut entry: Entry<RecordData>,
    force_update: bool,
    canonical: impl std::fmt::Display,
) -> Result<Entry<RecordData>, anyhow::Error> {
    let editor = Editor::new(EditorConfig { suffix: ".bib" });

    let data_changed = if let Some(new_entry) = editor.edit(&entry)? {
        let Entry {
            key: ref new_key,
            record_data: ref new_record_data,
        } = new_entry;

        if new_key != entry.key() {
            match Alias::from_str(new_key.as_ref()) {
                Ok(alias) => {
                    info!("Creating new alias '{alias}' for '{canonical}'");
                    if let Some(other_remote_id) = row.ensure_alias(&alias)? {
                        warn!("Alias '{alias}' already exists and refers to '{other_remote_id}'.");
                    }
                }
                Err(err) => {
                    error!("New key {} is not a valid alias: {err}.", new_key.as_ref());
                    suggest!("Any edits to the entry key are only used to create new aliases.");
                }
            }
        }

        let data_changed = new_record_data != entry.data();

        entry = new_entry;

        data_changed
    } else {
        set_failed();
        false
    };

    if data_changed || force_update {
        info!("Updating cached data for '{canonical}'");
        row.save_to_changelog()?;
        row.update_entry_data(&entry.record_data)?;
    }

    Ok(entry)
}

/// Merge an iterator of [`EntryData`] into existing data, using the merge rules as specified
/// by the passed [`UpdateMode`].
pub fn merge_record_data<'a, D: EntryData + 'a>(
    mode: UpdateMode,
    existing_record: &mut RecordData,
    new_raw_data: impl Iterator<Item = &'a D>,
    citation_key: impl std::fmt::Display,
) -> Result<(), MergeError> {
    match mode {
        UpdateMode::PreferCurrent => {
            info!("Updating {citation_key} with new data, skipping existing fields");
            for data in new_raw_data {
                existing_record.merge_or_skip(data);
            }
        }
        UpdateMode::PreferIncoming => {
            info!("Updating {citation_key} with new data, overwriting existing fields");
            for data in new_raw_data {
                existing_record.merge_or_overwrite(data);
            }
        }
        UpdateMode::Prompt => {
            info!("Updating {citation_key} with new data, prompting on conflict");
            for data in new_raw_data {
                existing_record.merge_with_callback(data, |key, current, incoming| {
                    eprintln!("Conflict for the field '{key}':");
                    eprintln!("   Current value: {current}");
                    eprintln!("  Incoming value: {incoming}");
                    let prompt = Input::new("Accept incoming value? [y]es / [N]o / [e]dit");
                    let choice = match prompt.input() {
                        Ok(r) => r,
                        Err(error) => {
                            error!("{error}");
                            warn!("Keeping current value for '{key}'");
                            return ConflictResolved::Current;
                        }
                    };

                    loop {
                        match choice.trim() {
                            "" => return ConflictResolved::Current,
                            c if "no".starts_with(c) || "NO".starts_with(c) => {
                                return ConflictResolved::Current
                            }
                            c if "yes".starts_with(c) || "YES".starts_with(c) => {
                                return ConflictResolved::Incoming
                            }
                            c if "edit".starts_with(c) || "EDIT".starts_with(c) => break,
                            _ => warn!("Invalid selection: {choice}!"),
                        }
                    }

                    let editor = Editor::new(EditorConfig { suffix: ".tex" });
                    let val = incoming.to_owned();
                    match editor.edit(&val) {
                        Ok(new) => ConflictResolved::New(new.unwrap_or(val)),
                        Err(error) => {
                            error!("{error}");
                            warn!("Keeping current value for '{key}'");
                            ConflictResolved::Current
                        }
                    }
                });
            }
        }
    }
    Ok(())
}
