use std::str::FromStr;

use anyhow::Result;

use super::OnConflict;

use crate::{
    db::state::{EntryRecordKey, State},
    entry::{ConflictResolved, EntryData, MutableEntryData},
    error::MergeError,
    logger::{error, info, reraise, suggest, warn},
    record::Alias,
    term::{Editor, EditorConfig, Input},
};

/// Given a candidate alias string, check if it is a valid alias, and if it is, try to add it as an
/// alias for the given row. If the alias does not exist, or it exists and points to the row, this
/// does not result in an error.
pub fn create_alias_if_valid(
    key: &str,
    row: &State<EntryRecordKey>,
) -> Result<(), rusqlite::Error> {
    match Alias::from_str(key) {
        Ok(alias) => {
            if let Some(other_remote_id) = row.ensure_alias(&alias)? {
                warn!("Alias '{alias}' already exists and refers to '{other_remote_id}'.");
            }
        }
        Err(err) => {
            error!("Bibtex key '{key}' is not a valid alias: {err}.");
            suggest!("Edits to the entry key are only used to create new aliases.");
        }
    }
    Ok(())
}

/// Merge an iterator of [`EntryData`] into existing data, using the merge rules as specified
/// by the passed [`OnConflict`].
pub fn merge_record_data<'a, D: EntryData + 'a>(
    on_conflict: OnConflict,
    existing_record: &mut MutableEntryData,
    new_raw_data: impl Iterator<Item = &'a D>,
    citation_key: impl std::fmt::Display,
) -> Result<(), MergeError> {
    match on_conflict {
        OnConflict::PreferCurrent => {
            info!("Updating {citation_key} with new data, skipping existing fields");
            for data in new_raw_data {
                existing_record.merge_or_skip(data);
            }
        }
        OnConflict::PreferIncoming => {
            info!("Updating {citation_key} with new data, overwriting existing fields");
            for data in new_raw_data {
                existing_record.merge_or_overwrite(data);
            }
        }
        OnConflict::Prompt => {
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
                            reraise(&error);
                            warn!("Keeping current value for '{key}'");
                            return ConflictResolved::Current;
                        }
                    };

                    loop {
                        match choice.trim() {
                            "" => return ConflictResolved::Current,
                            c if "no".starts_with(c) || "NO".starts_with(c) => {
                                return ConflictResolved::Current;
                            }
                            c if "yes".starts_with(c) || "YES".starts_with(c) => {
                                return ConflictResolved::Incoming;
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
                            reraise(&error);
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
