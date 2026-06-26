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

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use data_encoding::{BASE32, BASE32_NOPAD, Encoding};
use rapidhash::{v1::rapidhash_v1, v3::rapidhash_v3};

use crate::RemoteId;

/// Attachment directory format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentFormat {
    /// 0-padded zbmath identifiers, [`rapidhash::v1`], and padded base-32 encoding
    V0,
    /// Migrating from [`Self::V0`] to [`Self::V1`]
    V1Migrating,
    /// [`rapidhash::v3`] and unpadded base-32 encoding
    V1,
}

impl AttachmentFormat {
    fn fmt_dir(root: &Path) -> PathBuf {
        root.join(".autobib-format")
    }

    fn subdir_name(&self) -> &'static str {
        match self {
            Self::V0 => "v0",
            Self::V1Migrating => "v1-migrating",
            Self::V1 => "v1",
        }
    }

    fn subdir(&self, root: &Path) -> PathBuf {
        let mut fmt_dir = Self::fmt_dir(root);
        fmt_dir.push(self.subdir_name());
        fmt_dir
    }

    /// Read the attachment format, or return `None` if the attachment format subdirectory does not
    /// exist.
    fn read(root: &Path) -> Result<Option<Self>, anyhow::Error> {
        let mut fmt_dir = Self::fmt_dir(root);

        if !fmt_dir.try_exists()? {
            return Ok(None);
        }

        // first matching directory sets the format
        for variant in [Self::V0, Self::V1Migrating, Self::V1] {
            fmt_dir.push(variant.subdir_name());
            if fmt_dir.try_exists()? {
                return Ok(Some(variant));
            }
            fmt_dir.pop();
        }

        anyhow::bail!("attachment directory exists but is not in a recognized state")
    }
}

/// A read-only or read-write lock on the attachment folder.
///
/// Acquiring the lock requires writing a directory, and therefore cannot be done when Autobib is
/// run in read-only mode.
#[derive(Debug)]
pub struct AttachmentRootLock {
    root: PathBuf,
    read_only: bool,
}

impl Drop for AttachmentRootLock {
    fn drop(&mut self) {
        if !self.read_only {
            self.root.push(Self::lockdir());
            let _ = fs::remove_dir(&self.root);
        }
    }
}

impl AttachmentRootLock {
    fn lockdir() -> &'static str {
        "LOCKDIR"
    }

    pub(crate) fn cleanup(root: &Path) -> Result<(), anyhow::Error> {
        fs::remove_dir(root.join(Self::lockdir())).map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                anyhow::anyhow!("No lock directory to cleanup")
            } else {
                err.into()
            }
        })
    }

    fn acquire(root: PathBuf, read_only: bool) -> Result<Self, anyhow::Error> {
        let ld = root.join(Self::lockdir());
        if read_only {
            if ld.exists() {
                anyhow::bail!(
                    "Failed to acquire read lock in attachment directory.\n Retry later or clean up spurious locks with `autobib util cleanup-attachments --lockdir`"
                )
            } else {
                Ok(Self { root, read_only })
            }
        } else {
            fs::create_dir_all(&root)?;
            match fs::create_dir(&ld) {
                Ok(()) => Ok(Self { root, read_only }),
                Err(_) => anyhow::bail!(
                    "Failed to acquire read-write lock in attachment directory.\n Retry later or clean up spurious locks with `autobib util cleanup-attachments --lockdir`"
                ),
            }
        }
    }

    fn root(&self) -> &Path {
        &self.root
    }
}

/// The attachment directory root along with the format.
///
/// The attachment root uses a lock directory `$LOCKDIR`, which is an empty directory
/// `$AUTOBIB_ATTACHMENTS_DIRECTORY/LOCKDIR`. The attachment root can be initialized in read-only
/// mode or in read-write mode.
///
/// - In read-only mode, at startup the process checks for the presence of `$LOCKDIR` and aborts if
///   it exists. However, no further checks are made, and reads may be inconsistent with the actual
///   state of the attachments directory.
/// - In read-write mode, at initialization this method creates `$LOCKDIR` and deletes
///   `$LOCKDIR` when dropped.
///
/// Note that early versions of Autobib do not respect `$LOCKDIR`. Similarly, other non-Autobib
/// processes may not respect `$LOCKDIR` at all. Therefore, one must still ensure that mutable
/// operations never overwrite existing files.
#[derive(Debug)]
pub struct AttachmentRoot {
    lock: AttachmentRootLock,
    format: AttachmentFormat,
}

#[expect(unused)]
pub enum AttachmentRenameOutcome {
    /// The source attachment directory was missing.
    FromMissing(PathBuf, PathBuf),
    /// The target attachment directory already exists and is non-empty.
    ToExists(PathBuf, PathBuf),
    /// The directory was renamed successfully.
    Ok,
}

impl AttachmentRoot {
    /// Open an existing attachment root.
    pub fn open(root: PathBuf, read_only: bool) -> Result<Option<Self>, anyhow::Error> {
        Ok(if root.is_dir() {
            Some(Self::open_or_create(root, read_only)?)
        } else {
            None
        })
    }

    /// Acquire a lock (if necessary) and resolve the attachment format.
    pub fn open_or_create(root: PathBuf, read_only: bool) -> Result<Self, anyhow::Error> {
        let at_root = Self::resolve_unchecked(root, read_only)?;
        if at_root.format() == AttachmentFormat::V1Migrating {
            anyhow::bail!(
                "Attachment directory is currently being migrated. Resume with `autobib util migrate-attachments` in order to read and write to the directory."
            );
        }
        Ok(at_root)
    }

    /// Acquire a lock (if necessary) and resolve the attachment format, without checking if we are
    /// in a `migrating` state.
    pub fn resolve_unchecked(root: PathBuf, read_only: bool) -> Result<Self, anyhow::Error> {
        let lock = AttachmentRootLock::acquire(root, read_only)?;
        let format = if read_only {
            AttachmentFormat::read(lock.root())?.unwrap_or(AttachmentFormat::V0)
        } else {
            match AttachmentFormat::read(lock.root())? {
                Some(f) => f,
                None => {
                    let v0 = AttachmentFormat::V0.subdir(lock.root());
                    fs::create_dir_all(&v0)?;
                    AttachmentFormat::V0
                }
            }
        };
        Ok(Self { lock, format })
    }

    /// The format of the attachment directory on creation.
    pub fn format(&self) -> AttachmentFormat {
        self.format
    }

    /// The root attachments directory.
    pub fn dir(&self) -> &Path {
        self.lock.root()
    }

    /// Change the format.
    pub fn set_format(&mut self, new: AttachmentFormat) -> Result<(), anyhow::Error> {
        if new != self.format {
            let rt = self.lock.root();
            fs::rename(self.format.subdir(rt), new.subdir(rt))?;
            self.format = new;
        }

        Ok(())
    }

    /// Get the attachment directory corresponding to the identifier.
    pub fn attachment_dir(&self, id: &RemoteId) -> PathBuf {
        let mut path = PathBuf::new();
        self.attachment_dir_in(id, &mut path);
        path
    }

    /// Overwrite the provided buffer with the attachment directory corresponding to the identifier.
    ///
    /// This is useful for reducing allocations in case the caller already has a [`PathBuf`].
    pub fn attachment_dir_in(&self, id: &RemoteId, path: &mut PathBuf) {
        path.clear();
        path.push(self.lock.root());
        match self.format {
            AttachmentFormat::V0 => RemoteIdAttachmentPathV0(id).extend_attachments_path(path),
            AttachmentFormat::V1Migrating | AttachmentFormat::V1 => {
                RemoteIdAttachmentPathV1(id).extend_attachments_path(path);
            }
        }
    }

    /// Check whether the attachment directory corresponding to the identifier exists.
    pub fn exists(&self, id: &RemoteId) -> io::Result<bool> {
        self.attachment_dir(id).try_exists()
    }

    /// Try to rename the attachment directory at `from` to `to`.
    ///
    /// This will not overwrite the target directory.
    pub fn rename(
        &self,
        from: &RemoteId,
        to: &RemoteId,
    ) -> Result<AttachmentRenameOutcome, io::Error> {
        assert!(
            !self.lock.read_only,
            "Cannot rename file in read-only mode."
        );
        let source = self.attachment_dir(from);
        let target = self.attachment_dir(to);
        fs::create_dir_all(&target)?;

        match fs::rename(&source, &target) {
            Ok(()) => Ok(AttachmentRenameOutcome::Ok),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                Ok(AttachmentRenameOutcome::FromMissing(source, target))
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::AlreadyExists | io::ErrorKind::DirectoryNotEmpty
                ) =>
            {
                Ok(AttachmentRenameOutcome::ToExists(source, target))
            }
            Err(err) => Err(err),
        }
    }
}

/// A type which can be encoded as a platform-friendly path into a buffer.
pub trait PathHash {
    /// Extend the provided buffer with a hashed version of the path.
    fn extend_attachments_path(&self, path_buf: &mut PathBuf);
}

struct RemoteIdAttachmentPathV0<'a, S: AsRef<str>>(&'a RemoteId<S>);

struct RemoteIdAttachmentPathV1<'a, S: AsRef<str>>(&'a RemoteId<S>);

pub(crate) fn extend_attachment_path_v0<S: AsRef<str>>(id: &RemoteId<S>, path_buf: &mut PathBuf) {
    RemoteIdAttachmentPathV0(id).extend_attachments_path(path_buf);
}

pub(crate) fn extend_attachment_path_v1<S: AsRef<str>>(id: &RemoteId<S>, path_buf: &mut PathBuf) {
    RemoteIdAttachmentPathV1(id).extend_attachments_path(path_buf);
}

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
    encoding: Encoding,
) {
    let sub_id_hash: [u8; 8] = hash_fn(sub_id_bytes).to_le_bytes();

    let mut buffer = [0; 8];
    let input = &sub_id_hash[..4];
    let output = &mut buffer[0..encoding.encode_len(input.len())];
    let res = encoding.encode_mut_str(input, output);
    let sub_id_encoded: String = encoding.encode(sub_id_bytes);
    path_buf.extend([
        provider,
        &res[0..2],
        &res[2..4],
        &res[4..6],
        &sub_id_encoded,
    ]);
}

impl<S: AsRef<str>> PathHash for RemoteIdAttachmentPathV0<'_, S> {
    fn extend_attachments_path(&self, path_buf: &mut PathBuf) {
        let id = self.0;
        if id.provider() == "zbmath" && id.sub_id().len() < 8 {
            let mut padded_sub_id = [b'0'; 8];
            let sub_id = id.sub_id().as_bytes();
            padded_sub_id[8 - sub_id.len()..].copy_from_slice(sub_id);

            extend_hashed_path(path_buf, "zbmath", &padded_sub_id, rapidhash_v1, BASE32);
        } else {
            extend_hashed_path(
                path_buf,
                id.provider(),
                id.sub_id().as_bytes(),
                rapidhash_v1,
                BASE32,
            );
        }
    }
}

impl<S: AsRef<str>> PathHash for RemoteIdAttachmentPathV1<'_, S> {
    fn extend_attachments_path(&self, path_buf: &mut PathBuf) {
        let id = self.0;
        extend_hashed_path(
            path_buf,
            id.provider(),
            id.sub_id().as_bytes(),
            rapidhash_v3,
            BASE32_NOPAD,
        );
    }
}
