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
    fs::{self, File},
    io::{self, Seek, Write},
    path::{Path, PathBuf},
};

use data_encoding::{BASE32, BASE32_NOPAD, Encoding};
use rapidhash::{v1::rapidhash_v1, v3::rapidhash_v3};

use crate::{RemoteId, logger::info};

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
    pub const fn as_fmt_str(&self) -> &'static str {
        match self {
            Self::V0 => "v0",
            Self::V1Migrating => "v1-migrating",
            Self::V1 => "v1",
        }
    }
}

/// A lock on the attachment folder.
#[derive(Debug)]
pub struct AttachmentRootLock<const EXCLUSIVE: bool> {
    root: PathBuf,
    fmt: File,
}

impl<const EXCLUSIVE: bool> AttachmentRootLock<EXCLUSIVE> {
    fn fmt_file() -> &'static str {
        ".autobib-format"
    }

    fn format(&mut self) -> Result<AttachmentFormat, anyhow::Error> {
        use std::io::Read as _;
        let mut fmt_string = String::new();
        self.fmt.read_to_string(&mut fmt_string)?;
        match &fmt_string[..] {
            "" => Ok(AttachmentFormat::V0),
            "v1-migrating" => Ok(AttachmentFormat::V1Migrating),
            "v1" => Ok(AttachmentFormat::V1),
            unknown => anyhow::bail!(
                "Attachment directory is in unknown format '{unknown}'. The attachment directory may have been modified by a future version of Autobib."
            ),
        }
    }

    /// Acquire a lock to read or write to the attachment directory.
    fn acquire(root: PathBuf) -> Result<Self, io::Error> {
        let fmt = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(Self::fmt_file()))?;
        if EXCLUSIVE {
            fmt.lock()?;
        } else {
            fmt.lock_shared()?;
        };

        Ok(Self { root, fmt })
    }

    fn root(&self) -> &Path {
        &self.root
    }
}

impl AttachmentRootLock<false> {
    /// Upgrade to an exclusive lock
    fn upgrade(self) -> Result<AttachmentRootLock<true>, io::Error> {
        let Self { root, fmt } = self;
        fmt.lock()?;
        Ok(AttachmentRootLock { root, fmt })
    }
}

impl AttachmentRootLock<true> {
    fn set_format(&mut self, new: AttachmentFormat) -> Result<(), io::Error> {
        info!("Setting attachment root format to '{}", new.as_fmt_str());
        self.fmt.set_len(0)?;
        self.fmt.rewind()?;
        self.fmt.write_all(new.as_fmt_str().as_bytes())?;
        Ok(())
    }
}

/// The attachment directory root along with the format.
///
/// The attachment root uses [filesystem locks](std::fs::File::lock) on the attachment format file
/// `$AUTOBIB_ATTACHMENTS_DIRECTORY/.autobib-format`. These locks are advisory, and other Autobib
/// processes use them to coordinate.
///
/// Note that early versions of Autobib do not respect the format file or the advisory locks.
/// Therefore, one must still ensure that mutable operations never overwrite existing files.
#[derive(Debug)]
pub struct AttachmentRoot<const EXCLUSIVE: bool> {
    lock: AttachmentRootLock<EXCLUSIVE>,
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

impl<const EXCLUSIVE: bool> AttachmentRoot<EXCLUSIVE> {
    /// Acquire a lock (if necessary) and resolve the attachment format.
    pub fn open_or_create_unchecked(root: PathBuf) -> Result<Self, anyhow::Error> {
        fs::create_dir_all(&root)?;
        let mut lock = AttachmentRootLock::acquire(root)?;
        let format = lock.format()?;
        Ok(Self { lock, format })
    }
}

impl AttachmentRoot<false> {
    /// Open an existing attachment root.
    pub fn open(root: PathBuf) -> Result<Option<Self>, anyhow::Error> {
        Ok(if root.is_dir() {
            Some(Self::open_or_create(root)?)
        } else {
            None
        })
    }

    /// Acquire a lock (if necessary) and resolve the attachment format.
    pub fn open_or_create(root: PathBuf) -> Result<Self, anyhow::Error> {
        let at_root = Self::open_or_create_unchecked(root)?;
        if at_root.format == AttachmentFormat::V1Migrating {
            anyhow::bail!(
                "Attachment directory is currently being migrated. Resume with `autobib clean attachments --migrate` in order to read and write to the directory."
            );
        }
        Ok(at_root)
    }
}

impl AttachmentRoot<false> {
    #[expect(unused)]
    pub fn upgrade(self) -> io::Result<AttachmentRoot<true>> {
        let Self { lock, format } = self;
        Ok(AttachmentRoot {
            lock: lock.upgrade()?,
            format,
        })
    }
}

impl AttachmentRoot<true> {
    /// Change the format.
    pub fn set_format(&mut self, new: AttachmentFormat) -> Result<(), io::Error> {
        self.lock.set_format(new)?;
        self.format = new;
        Ok(())
    }
}

impl<const EXCLUSIVE: bool> AttachmentRoot<EXCLUSIVE> {
    /// The format of the attachment directory on creation.
    pub fn format(&self) -> AttachmentFormat {
        self.format
    }

    /// The root attachments directory.
    pub fn dir(&self) -> &Path {
        self.lock.root()
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
/// applied to the format-specific sub-id into little endian bytes, then taking the four least
/// significant bytes (in decreasing order), encoding using BASE32 into 7 ASCII characters, and then
/// taking the first 6.
///
/// The header `xx/xx/xx` ensures that each directory does not have more than 1024 immediate
/// sub-directories.
fn extend_hashed_path<'a, H: FnOnce(&'a [u8]) -> u64>(
    path_buf: &mut PathBuf,
    provider: &str,
    sub_id_bytes: &'a [u8],
    hash_fn: H,
    encoding: Encoding,
) {
    let sub_id_hash: [u8; 8] = hash_fn(sub_id_bytes).to_le_bytes();

    let mut buffer = [0; 8];
    let input = &sub_id_hash[..4];
    // FIXME: 8 with BASE32, 7 with BASE32_NOPAD; const-ify this later once we
    // no longer need BASE32
    let output = &mut buffer[..encoding.encode_len(input.len())];
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
