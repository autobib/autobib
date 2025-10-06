use std::{
    fs::read_to_string,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::bail;

use crate::{
    entry::{Entry, RecordData},
    http::Client,
    logger::info,
    path_hash::PathHash,
    record::{RecursiveRemoteResponse, RemoteId, get_remote_response_recursive},
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

/// Either obtain data from a `.bib` file at the provided path, or look up data from the
/// provider.
pub fn data_from_path_or_remote<P: AsRef<Path>, C: Client>(
    maybe_path: Option<P>,
    remote_id: RemoteId,
    client: &C,
) -> Result<(RecordData, RemoteId), anyhow::Error> {
    match maybe_path {
        Some(path) => Ok((data_from_path(path)?, remote_id)),
        _ => match get_remote_response_recursive(remote_id, client)? {
            RecursiveRemoteResponse::Exists(record_data, canonical) => Ok((record_data, canonical)),
            RecursiveRemoteResponse::Null(null_remote_id) => {
                bail!("Remote data for canonical id '{null_remote_id}' is null");
            }
        },
    }
}

/// Either obtain data from a `.bib` file at the provided path, or return the default data.
pub fn data_from_path_or_default<P: AsRef<Path>>(
    maybe_path: Option<P>,
) -> Result<RecordData, anyhow::Error> {
    match maybe_path {
        Some(path) => data_from_path(path),
        _ => Ok(RecordData::default()),
    }
}

/// Obtain data from a bibtex record at a provided path.
fn data_from_path<P: AsRef<Path>>(path: P) -> Result<RecordData, anyhow::Error> {
    let bibtex = read_to_string(path)?;
    let entry = Entry::<RecordData>::from_str(&bibtex)?;
    Ok(entry.record_data)
}
