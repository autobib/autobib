use std::{path::PathBuf, str::FromStr};

use anyhow::Result;

use super::OnConflict;

use crate::{
    app::data_from_path,
    db::{
        Identifier,
        state::{IsEntry, RecordsInsert, State},
    },
    entry::{
        ConflictResolved, Entry, EntryData, EntryEditCommand, EntryKey, MutableEntryData,
        RawEntryData,
    },
    error::MergeError,
    logger::{error, info, reraise, set_failed, suggest, warn},
    normalize::{Normalization, Normalize},
    record::{Alias, RemoteId},
    term::{Editor, EditorConfig, Input},
};

/// Given a candidate alias string, check if it is a valid alias, and if it is, try to add it as an
/// alias for the given row. If the alias does not exist, or it exists and points to the row, this
/// does not result in an error.
pub fn create_alias_if_valid(key: &str, row: &State<IsEntry>) -> Result<(), rusqlite::Error> {
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

/// Insert new data for the given state.
///
/// If data is not available at the provided path, prompt the user for data.
///
/// If the row does not exist in the 'Records' table, this inserts a new row as the unique entry.
/// If the row exists, this adds the new row as a child of the existing row.
pub fn insert<'conn, I>(
    missing: State<'conn, I>,
    from_bibtex: Option<PathBuf>,
    remote_id: &RemoteId,
    no_interactive: bool,
    normalization: &Normalization,
    edit: &EntryEditCommand,
    add_alias: Option<&Alias>,
) -> anyhow::Result<()>
where
    State<'conn, I>: RecordsInsert<'conn>,
{
    let exists = if let Some(path) = from_bibtex {
        let mut data = data_from_path(path)?;
        data.normalize(normalization);
        missing.insert(&RawEntryData::from_entry_data(&data), remote_id)?
    } else if !edit.is_identity() {
        let mut data = MutableEntryData::default();
        data.edit(edit);
        missing.insert(&RawEntryData::from_entry_data(&data), remote_id)?
    } else if no_interactive {
        let data = MutableEntryData::<&'static str>::default();
        warn!("Inserting local data with no contents in non-interactive mode");
        missing.insert(&RawEntryData::from_entry_data(&data), remote_id)?
    } else {
        let record_data = MutableEntryData::<String>::default();
        let entry = Entry {
            key: EntryKey::try_new(remote_id.name().into())
                .unwrap_or_else(|_| EntryKey::placeholder()),
            record_data,
        };

        if let Some(Entry { key, record_data }) = Editor::new_bibtex().edit(&entry)? {
            let row = missing.insert(&RawEntryData::from_entry_data(&record_data), remote_id)?;
            if key.as_ref() != remote_id.name() {
                create_alias_if_valid(key.as_ref(), &row)?;
            }
            row.commit()?;
        } else {
            missing.commit()?;
            set_failed();
        }
        return Ok(());
    };

    if let Some(alias) = add_alias
        && !exists.add_alias(alias)?
    {
        error!("Alias '{alias}' already exists and references a different record.");
    }

    exists.commit()?;
    Ok(())
}

/// Merge an iterator of [`EntryData`] into existing data, using the merge rules as specified
/// by the passed [`OnConflict`].
pub fn merge_record_data<'a, D: EntryData + 'a>(
    on_conflict: OnConflict,
    existing_record: &mut MutableEntryData,
    new_raw_data: impl Iterator<Item = &'a D>,
    id_display: impl std::fmt::Display,
) -> Result<(), MergeError> {
    match on_conflict {
        OnConflict::PreferCurrent => {
            info!("Updating {id_display} with new data, skipping existing fields");
            for data in new_raw_data {
                existing_record.merge_or_skip(data);
            }
        }
        OnConflict::PreferIncoming => {
            info!("Updating {id_display} with new data, overwriting existing fields");
            for data in new_raw_data {
                existing_record.merge_or_overwrite(data);
            }
        }
        OnConflict::Prompt => {
            info!("Updating {id_display} with new data, prompting on conflict");
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
