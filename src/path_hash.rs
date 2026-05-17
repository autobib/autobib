// # Migrating attachments
//
// Currently, the attachment directory format is changing. Eventually, the plan is for the format
// to be fixed, so a lot of the logic here will become unnecessary, but in the meantime here is
// a brief explanation of the format.
//
// A directory `$AUTOBIB_ATTACHMENTS_DIRECTORY/.autobib-format` contains marker directories which
// describe the current state. The first case below to match determines the behaviour:
//
// 1. `.autobib-format` does not exist, or `.autobib-format/v0` exists: this is the legacy
//    attachment format, using `rapidhash::v1` and `zbmath` identifiers with 0-padding.
// 2. `.autobib-format/v1-migrating` exists: migration from `v0` to `v1` was interrupted, so the
//    directories are mixed between the `v0` and `v1` formats.
// 3. `.autobib-format/v1` exists: this is the new attachment format, using
//    `rapidhash::v3` and `zbmath` identifiers without 0-padding.
// 4. Else: the attachment format is unknown to the current binary, resulting in an error
//
// When migrating:
//
// 1. Run `rmdir .autobib-format/v0`:
//    - If successful: this puts the directory into an invalid state, so other `autobib` instances do not try
//        to read.
//    - If this fails: the version is not `v0` so migration should not be performed; bail.
// 2. Create `.autobib-format/v1-migrating`.
// 3. Perform the migration.
// 4. Create `.autobib-format/v2`.
// 5. Delete `.autobib-format/v1-migrating`.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

use data_encoding::BASE32;
use rapidhash::{v1::rapidhash_v1, v3::rapidhash_v3};

use crate::RemoteId;

/// Attachment directory format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentFormat {
    /// 0-padded zbmath identifiers and [`rapidhash::v1`]
    V0,
    /// [`rapidhash::v3`]
    V1,
}

/// The attachment directory root along with the format.
#[derive(Debug, Clone)]
pub struct AttachmentRoot {
    root: PathBuf,
    format: AttachmentFormat,
}

impl AttachmentRoot {
    pub fn resolve(root: PathBuf, read_only: bool) -> Result<Self, anyhow::Error> {
        let format = AttachmentFormat::resolve(&root, read_only)?;
        Ok(Self { root, format })
    }

    pub fn attachment_dir(&self, id: &RemoteId) -> PathBuf {
        let mut path = PathBuf::new();
        self.attachment_dir_in(id, &mut path);
        path
    }

    /// Overwrite the provided buffer with the attachment directory corresponding to the identifier.
    pub fn attachment_dir_in(&self, id: &RemoteId, path: &mut PathBuf) {
        path.clear();
        path.push(&self.root);
        match self.format {
            AttachmentFormat::V0 => RemoteIdAttachmentPathV0(id).extend_attachments_path(path),
            AttachmentFormat::V1 => RemoteIdAttachmentPathV1(id).extend_attachments_path(path),
        }
    }
}

impl AttachmentFormat {
    /// Determine the attachment format using a marker directory.
    ///
    /// If the format marker does not exist, initialize it as `v0` (unless `read_only`)
    pub fn resolve(attachment_root: &Path, read_only: bool) -> Result<Self, anyhow::Error> {
        const FORMAT_DIR: &str = ".autobib-format";
        const FORMAT_V0: &str = "v0";
        const FORMAT_V1_MIGRATING: &str = "v1-migrating";
        const FORMAT_V1: &str = "v1";

        let format_root = attachment_root.join(FORMAT_DIR);

        // fast-path checks: look for a matching format directory
        if format_root.join(FORMAT_V0).is_dir() {
            Ok(Self::V0)
        } else if format_root.join(FORMAT_V1_MIGRATING).is_dir() {
            anyhow::bail!(
                "The attachment directory format is being migrated by a future version of autobib. \
                 The migration process was interrupted. Complete the migration with that binary, \
                 and this version can read the attachment directory afterwards."
            );
        } else if format_root.join(FORMAT_V1).is_dir() {
            Ok(Self::V1)
        } else {
            // we did not find any of the `v0`, `v1-migrating`, or `v1` directories, so there
            // are two cases:
            // 1. The `.autobib-format` directory itself does not yet exist, in which case we
            //    initialize with the `v0` file
            // 2. The format is incompatible.
            if read_only {
                if !format_root.exists() {
                    return Ok(Self::V0);
                }
            } else {
                fs::create_dir_all(attachment_root)?;

                match fs::create_dir(&format_root) {
                    Ok(()) => {
                        fs::create_dir(format_root.join(FORMAT_V0))?;
                        return Ok(Self::V0);
                    }
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(err) => return Err(err.into()),
                }
            }

            anyhow::bail!("The attachment directory uses an incompatible attachment format.");
        }
    }
}

/// A type which can be encoded as a platform-friendly path into a buffer.
pub trait PathHash {
    /// Extend the provided buffer with a hashed version of the path.
    fn extend_attachments_path(&self, path_buf: &mut PathBuf);
}

struct RemoteIdAttachmentPathV0<'a>(&'a RemoteId);

struct RemoteIdAttachmentPathV1<'a>(&'a RemoteId);

/// In order to reduce the number of files which are in the same directory, we apply a 30-bit
/// header to each path, which is encoded in base32 as `xx/xx/xx`. Then the corresponding path is:
/// ```text
/// provider/xx/xx/xx/base32-encoding-of-sub-id/
/// ```
///
/// The 30 bit header is formed by converting the u64 output of the relevant rapidhash algorithm
/// applied to the format-specific sub-id into little endian bytes, then taking the four most
/// significant bytes (in decreasing order), encoding using BASE32 into 8 ASCII characters, and then
/// taking the first 6.
///
/// The header `xx/xx/xx` ensures that each directory does not have more than 1024 immediate
/// sub-directories.
fn extend_hashed_path<H: for<'a> FnOnce(&'a [u8]) -> u64>(
    path_buf: &mut PathBuf,
    provider: &str,
    sub_id_bytes: &[u8],
    hash_fn: H,
) {
    let sub_id_hash: [u8; 8] = hash_fn(sub_id_bytes).to_le_bytes();

    let mut buffer = [0; 8];
    let res = BASE32.encode_mut_str(&sub_id_hash[..4], &mut buffer);
    let sub_id_encoded: String = BASE32.encode(sub_id_bytes);
    path_buf.extend([
        provider,
        &res[0..2],
        &res[2..4],
        &res[4..6],
        &sub_id_encoded,
    ]);
}

impl PathHash for RemoteIdAttachmentPathV0<'_> {
    fn extend_attachments_path(&self, path_buf: &mut PathBuf) {
        let id = self.0;
        if id.provider() == "zbmath" && id.sub_id().len() < 8 {
            let mut padded_sub_id = [b'0'; 8];
            let sub_id = id.sub_id().as_bytes();
            padded_sub_id[8 - sub_id.len()..].copy_from_slice(sub_id);

            extend_hashed_path(path_buf, "zbmath", &padded_sub_id, rapidhash_v1);
        } else {
            extend_hashed_path(
                path_buf,
                id.provider(),
                id.sub_id().as_bytes(),
                rapidhash_v1,
            );
        }
    }
}

impl PathHash for RemoteIdAttachmentPathV1<'_> {
    fn extend_attachments_path(&self, path_buf: &mut PathBuf) {
        let id = self.0;
        extend_hashed_path(
            path_buf,
            id.provider(),
            id.sub_id().as_bytes(),
            rapidhash_v3,
        );
    }
}
