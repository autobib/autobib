mod migrate;

use std::path::{Path, PathBuf};

use crate::{logger::info, path_hash::AttachmentRoot, record::RemoteId};
pub use migrate::migrate_attachments;

/// Get the attachment root directory, either as a default from the data directory or using the
/// user provided value.
pub fn get_attachment_root(
    data_dir: &Path,
    default_attachments_dir: Option<PathBuf>,
    read_only: bool,
) -> Result<AttachmentRoot, anyhow::Error> {
    let root = get_attachment_root_path(data_dir, default_attachments_dir);
    AttachmentRoot::resolve(root, read_only)
}

/// Get the attachment root directory path, either as a default from the data directory or using the
/// user provided value.
pub fn get_attachment_root_path(
    data_dir: &Path,
    default_attachments_dir: Option<PathBuf>,
) -> PathBuf {
    // Initialize the file directory path
    if let Some(file_dir) = default_attachments_dir {
        // at a user-provided path
        info!(
            "Using user-provided attachment directory '{}'",
            file_dir.display()
        );
        file_dir
    } else {
        // at the default path
        let default_attachments_path = data_dir.join("attachments");
        info!(
            "Using default attachment directory '{}'",
            default_attachments_path.display()
        );

        default_attachments_path
    }
}

/// Get the attachment directory corresponding to the provided identifier.
pub fn get_attachment_dir(
    data_dir: &Path,
    default_attachments_dir: Option<PathBuf>,
    read_only: bool,
    canonical: &RemoteId,
) -> Result<PathBuf, anyhow::Error> {
    let attachments_root = get_attachment_root(data_dir, default_attachments_dir, read_only)?;
    Ok(attachments_root.attachment_dir(canonical))
}
