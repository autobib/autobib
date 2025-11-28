use std::{
    fs::read_to_string,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::bail;

use crate::{
    Config,
    db::{RecordDatabase, state::RecordIdState},
    entry::{Entry, MutableEntryData},
    logger::info,
    path_hash::PathHash,
    record::{RecordId, RemoteId},
};

/// Get the attachment root directory, either as a default from the data directory or using the
/// user provided value.
pub fn get_attachment_root(
    data_dir: &Path,
    default_attachments_dir: Option<PathBuf>,
) -> Result<PathBuf, anyhow::Error> {
    // Initialize the file directory path
    Ok(if let Some(file_dir) = default_attachments_dir {
        // at a user-provided path
        info!(
            "Using user-provided file directory '{}'",
            file_dir.display()
        );
        file_dir
    } else {
        // at the default path
        let default_attachments_path = data_dir.join("attachments");
        info!(
            "Using default file directory '{}'",
            default_attachments_path.display()
        );

        default_attachments_path
    })
}

/// Get the attachment directory corresponding to the provided citation key.
pub fn get_attachment_dir(
    data_dir: &Path,
    default_attachments_dir: Option<PathBuf>,
    canonical: &RemoteId,
) -> Result<PathBuf, anyhow::Error> {
    let mut attachments_root = get_attachment_root(data_dir, default_attachments_dir)?;
    canonical.extend_attachments_path(&mut attachments_root);
    Ok(attachments_root)
}

pub fn data_from_key<F: FnOnce() -> Vec<(regex::Regex, String)>>(
    record_db: &mut RecordDatabase,
    record_id: RecordId,
    cfg: &Config<F>,
) -> Result<MutableEntryData, anyhow::Error> {
    match record_db.state_from_record_id(record_id, &cfg.alias_transform)? {
        RecordIdState::Entry(_, entry_row_data, state) => {
            state.commit()?;
            Ok(MutableEntryData::from_entry_data(&entry_row_data.data))
        }
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

/// Obtain data from a bibtex record at a provided path.
pub fn data_from_path<P: AsRef<Path>>(path: P) -> Result<MutableEntryData, anyhow::Error> {
    let bibtex = read_to_string(path)?;
    let entry = Entry::<MutableEntryData>::from_str(&bibtex)?;
    Ok(entry.record_data)
}
