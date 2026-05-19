use std::{
    fs,
    io::{self, ErrorKind},
    path::Path,
};

use walkdir::WalkDir;

/// Remove empty directories recursively under `root`.
///
/// `max_depth` is relative to `root`. For example, `0` checks only if `root` is empty, `1` checks
/// `root` and its immediate children, etc.
pub fn remove_empty_dirs(root: &Path, max_depth: usize) -> io::Result<()> {
    for entry in WalkDir::new(root)
        .max_depth(max_depth)
        .contents_first(true)
        .follow_links(false)
        .follow_root_links(false)
    {
        let entry = entry?;

        if !entry.file_type().is_dir() {
            continue;
        }

        match fs::remove_dir(entry.path()) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::DirectoryNotEmpty => {}
            Err(err) if err.kind() == ErrorKind::NotFound => {
                // likely removed in a different process
            }
            Err(err) => return Err(err),
        }
    }

    Ok(())
}

/// Delete empty subdirectories inside the attachments folder.
///
/// This deletes all empty directories inside subdirectories with an ascii alphabetic filename, to a
/// maximum depth of 4 to avoid deletion of user directories (even if empty).
pub fn cleanup_empty_attachment_dirs(attachment_root: &Path) -> io::Result<()> {
    for entry in fs::read_dir(attachment_root)? {
        let entry = entry?;

        // providers are always ascii alphabetic
        if entry.file_type()?.is_dir()
            && entry
                .file_name()
                .as_os_str()
                .to_str()
                .is_some_and(|s| s.bytes().all(|b| b.is_ascii_alphabetic()))
        {
            // depth 4: provider/xx/xx/xx/base32/
            remove_empty_dirs(&entry.path(), 4)?;
        }
    }

    Ok(())
}
