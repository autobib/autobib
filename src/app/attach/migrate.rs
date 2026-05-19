use std::{
    fs, io,
    path::{Path, PathBuf},
};

use data_encoding::BASE32;
use walkdir::{DirEntry, WalkDir};

use crate::{
    RemoteId,
    logger::{error, warn},
    path_hash::{
        FORMAT_DIR, FORMAT_V0, FORMAT_V1, FORMAT_V1_MIGRATING, extend_attachment_path_v0,
        extend_attachment_path_v1,
    },
};

/// Migrate attachment directory from v0 to v1
pub fn migrate_attachments(attachment_root: &Path) -> Result<(), anyhow::Error> {
    init_migration(attachment_root)?;

    let migrations = migration_candidates(attachment_root)?;
    let mut failed = false;
    for (source, target) in migrations {
        if !migrate_attachment_dir(&source, &target)? {
            failed = true;
        }
    }

    if failed {
        anyhow::bail!(
            "Attachment migration failed. Resolve the errors above, then rerun `autobib util migrate-attachments`."
        );
    }

    finish_migration(attachment_root)?;
    Ok(())
}

/// Determine a list of candidate migrations to perform by walking the attachment directory.
fn migration_candidates(attachment_root: &Path) -> Result<Vec<(PathBuf, PathBuf)>, anyhow::Error> {
    let mut migrations = Vec::new();

    // nothing to do
    if !attachment_root.exists() {
        return Ok(migrations);
    }

    // walk all directories which look like a valid attachment directory
    for entry in WalkDir::new(attachment_root)
        .min_depth(5)
        .max_depth(5)
        .into_iter()
        .filter_entry(is_attachment_walk_entry)
    {
        let source = entry?.into_path();

        // obtain the remote-id from the attachment folder
        let Some(provider) = source
            .strip_prefix(attachment_root)?
            .components()
            .next()
            .and_then(|component| component.as_os_str().to_str())
        else {
            continue;
        };
        let Some(id) = decode_attachment_dir_id(provider, &source) else {
            continue;
        };

        // construct old and new attachment paths
        let mut old_path = PathBuf::new();
        old_path.push(attachment_root);
        extend_attachment_path_v0(&id, &mut old_path);

        let mut new_path = PathBuf::new();
        new_path.push(attachment_root);
        extend_attachment_path_v1(&id, &mut new_path);

        if source == old_path && source != new_path {
            migrations.push((source, new_path));
        } else if source != new_path {
            // skip matches on the new path, since these were already migrated
            warn!(
                "Skipping invalid attachment directory: {}",
                source.display()
            );
        }
    }
    Ok(migrations)
}

/// Returns if a directory entry is in the format `provider/xx/xx/xx/base32-encoded-sub-id/`.
fn is_attachment_walk_entry(entry: &DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }

    match entry.depth() {
        0 => true,
        1 => entry
            .file_name()
            .to_str()
            .is_some_and(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase())),
        2..=4 => entry.file_name().to_str().is_some_and(|name| {
            matches!(
                name.as_bytes(),
                [b'A'..=b'Z' | b'2'..=b'7', b'A'..=b'Z' | b'2'..=b'7']
            )
        }),
        5 => true, // base-32 check happens when decoding later on
        _ => false,
    }
}

fn decode_attachment_dir_id(provider: &str, source: &Path) -> Option<RemoteId> {
    if let Some(encoded) = source.file_name().and_then(|name| name.to_str())
        && let Ok(sub_id_bytes) = BASE32.decode(encoded.as_bytes())
        && let Ok(sub_id) = std::str::from_utf8(&sub_id_bytes)
        && let Ok(id) = RemoteId::from_parts(provider, sub_id)
    {
        Some(id)
    } else {
        warn!(
            "Skipping invalid attachment directory: {}",
            source.display()
        );
        None
    }
}

fn init_migration(attachment_root: &Path) -> Result<(), anyhow::Error> {
    let format_root = attachment_root.join(FORMAT_DIR);
    let v0 = format_root.join(FORMAT_V0);
    let migrating = format_root.join(FORMAT_V1_MIGRATING);
    let v1 = format_root.join(FORMAT_V1);

    match fs::remove_dir(&v0) {
        Ok(()) => {
            // `v0` exists and we just deleted it, so create `v1-migrating` to indicate that
            // migration is in progress
            fs::create_dir_all(&migrating)?;
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            // `v0` does not exist but `v1-migrating` does; currently migrating
            if migrating.is_dir() {
                Ok(())
            } else if v1.is_dir() {
                // already migrated
                anyhow::bail!("Attachment directory already uses the v1 attachment format.");
            } else {
                // check if the format directory exists at all
                match fs::create_dir(&format_root) {
                    Ok(()) => {
                        // it does not, so we assume it is currently in `v0` format
                        fs::create_dir(&migrating)?;
                        Ok(())
                    }
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                        // it exists, but none of the expected checks could determine the format
                        anyhow::bail!(
                            "The attachment directory is in an unrecognizable format or is currently being edited by another program."
                        );
                    }
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {
                        anyhow::bail!(
                            "The provided attachment directory is empty; no attachment migration is necessary."
                        );
                    }
                    Err(err) => Err(err.into()),
                }
            }
        }
        Err(err) => Err(err.into()),
    }
}

fn migrate_attachment_dir(source: &Path, target: &Path) -> Result<bool, anyhow::Error> {
    if source == target {
        return Ok(true);
    }

    match fs::read_dir(source) {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotADirectory => {
            warn!(
                "Skipping invalid path which is not a directory: {}",
                source.display()
            );
        }
        Err(err) => return Err(err.into()),
    };

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    if target.exists() {
        error!(
            "Target directory already exists.\n\
             source: {}\n\
             target: {}\n\
             Merge, remove, or rename one of these directories, then rerun `autobib util migrate-attachments`.",
            source.display(),
            target.display()
        );
        return Ok(false);
    }

    fs::rename(source, target)?;
    Ok(true)
}

fn finish_migration(attachment_root: &Path) -> Result<(), anyhow::Error> {
    let format_root = attachment_root.join(FORMAT_DIR);

    fs::create_dir_all(format_root.join(FORMAT_V1))?;
    fs::remove_dir(format_root.join(FORMAT_V1_MIGRATING))?;
    Ok(())
}
