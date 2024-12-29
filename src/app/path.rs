use std::{
    fs::read_to_string,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::bail;

use crate::{
    db::{RawRecordData, RecordData},
    entry::Entry,
    http::HttpClient,
    logger::info,
    path_hash::PathHash,
    record::{get_remote_response_recursive, RecursiveRemoteResponse, RemoteId},
};

/// Obtain the attachment directory corresponding to the provided citation key, with automatic
/// record retrieval.
pub fn get_attachment_dir(
    canonical: &RemoteId,
    data_dir: &Path,
    default_attachments_dir: Option<PathBuf>,
) -> Result<PathBuf, anyhow::Error> {
    // Initialize the file directory path
    let mut attachments_dir = if let Some(file_dir) = default_attachments_dir {
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
    };

    canonical.extend_attachments_path(&mut attachments_dir);

    Ok(attachments_dir)
}

/// Either obtain data from a `.bib` file at the provided path, or look up data from the
/// provider.
pub fn data_from_path_or_remote<P: AsRef<Path>>(
    maybe_path: Option<P>,
    remote_id: RemoteId,
    client: &HttpClient,
) -> Result<(RawRecordData, RemoteId), anyhow::Error> {
    if let Some(path) = maybe_path {
        Ok((data_from_path(path)?, remote_id))
    } else {
        match get_remote_response_recursive(remote_id, client)? {
            RecursiveRemoteResponse::Exists(record_data, canonical) => {
                Ok((RawRecordData::from(&record_data), canonical))
            }
            RecursiveRemoteResponse::Null(null_remote_id) => {
                bail!("Remote data for canonical id '{null_remote_id}' is null");
            }
        }
    }
}

/// Either obtain data from a `.bib` file at the provided path, or return the default data.
pub fn data_from_path_or_default<P: AsRef<Path>>(
    maybe_path: Option<P>,
) -> Result<RawRecordData, anyhow::Error> {
    if let Some(path) = maybe_path {
        data_from_path(path)
    } else {
        Ok((&RecordData::default()).into())
    }
}

/// Obtain data from a bibtex record at a provided path.
fn data_from_path<P: AsRef<Path>>(path: P) -> Result<RawRecordData, anyhow::Error> {
    let bibtex = read_to_string(path)?;
    let entry = Entry::<RawRecordData>::from_str(&bibtex)?;
    Ok(entry.record_data)
}
