// # Migrating attachments
//
// Currently, the attachment directory format is changing. Eventually, the plan is for the format
// to be fixed, so a lot of the logic here will become unnecessary, but in the meantime here is
// a brief explanation of the format.
//
// A file `$AUTOBIB_ATTACHMENTS_DIRECTORY/.autobib_lock` is used for process synchronization and to
// define a small amount of current state. Currently, the file content is used to define the format:
//
// 1. does not exist, or is empty: this is the legacy attachment format, using
//    `rapidhash::v1`, `zbmath` identifiers with 0-padding, and padded base-32 encoding.
// 2. `v1-migrating`: migration from `v0` to `v1` was interrupted, so the
//    directories are mixed between the `v0` and `v1` formats.
// 3. `v1`: this is the new attachment format, using `rapidhash::v3`, `zbmath` identifiers without
//    0-padding, and unpadded base-32 encoding.
// 4. Else: the attachment format is unknown to the current binary, resulting in an error

use std::{
    fs::{self, File},
    io::{self, Seek, Write},
    path::{Path, PathBuf},
};

use data_encoding::{BASE32, BASE32_NOPAD, Encoding};
use rapidhash::{v1::rapidhash_v1, v3::rapidhash_v3};

use crate::{Identifier, logger::info};

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
    // if opened in read-only mode, the format may file may not exist
    // in which case this is `None`
    // FIXME: when we remove the format spec, this should just be a `File`
    // and non-existence of the `File` means the attachment directory is empty
    fmt: Option<File>,
}

impl<const EXCLUSIVE: bool> AttachmentRootLock<EXCLUSIVE> {
    fn fmt_file() -> &'static str {
        ".autobib_lock"
    }

    fn format(&mut self) -> Result<AttachmentFormat, anyhow::Error> {
        use std::io::Read as _;
        let mut fmt_string = String::new();
        if let Some(ref mut f) = self.fmt {
            f.read_to_string(&mut fmt_string)?;
            match &fmt_string[..] {
                "" => Ok(AttachmentFormat::V0),
                "v1-migrating" => Ok(AttachmentFormat::V1Migrating),
                "v1" => Ok(AttachmentFormat::V1),
                unknown => anyhow::bail!(
                    "Attachment directory is in unknown format '{unknown}'. The attachment directory may have been modified by a future version of Autobib."
                ),
            }
        } else {
            Ok(AttachmentFormat::V0)
        }
    }

    /// Acquire a lock to read or write to the attachment directory.
    fn acquire(root: PathBuf, read_only: bool) -> Result<Self, io::Error> {
        if read_only && EXCLUSIVE {
            panic!("Tried to open attachment directory with exclusive lock in read-only mode!")
        }

        let fmt = if read_only {
            match std::fs::OpenOptions::new()
                .read(true)
                .open(root.join(Self::fmt_file()))
            {
                Ok(f) => Some(f),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    // FIXME: maybe this warning is worth including; think about it
                    // warn!("Failed to acquire shared attachment lock in `--read-only` mode since the attachment directory was never opened with write permission.\n      Reads may be invalid if the attachment directory is concurrently modified.");
                    None
                }
                Err(err) => return Err(err),
            }
        } else {
            Some(
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(root.join(Self::fmt_file()))?,
            )
        };

        if let Some(ref f) = fmt {
            if EXCLUSIVE {
                f.lock()?;
            } else {
                f.lock_shared()?;
            };
        }

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
        fmt.as_ref()
            .expect("Tried to upgrade a read-only lock!")
            .lock()?;
        Ok(AttachmentRootLock { root, fmt })
    }
}

impl AttachmentRootLock<true> {
    fn set_format(&mut self, new: AttachmentFormat) -> Result<(), io::Error> {
        info!("Setting attachment root format to '{}'", new.as_fmt_str());
        let f = self
            .fmt
            .as_mut()
            .expect("Tried to change the format of a read-only attachment directory!");
        f.set_len(0)?;
        f.rewind()?;
        f.write_all(new.as_fmt_str().as_bytes())?;
        Ok(())
    }
}

/// The attachment directory root along with the format.
///
/// The attachment root uses [filesystem locks](std::fs::File::lock) on the attachment format file
/// `$AUTOBIB_ATTACHMENTS_DIRECTORY/.autobib_lock`. These locks are advisory, and other Autobib
/// processes use them to coordinate.
///
/// Note that early versions of Autobib do not respect the format file or the advisory locks.
/// Therefore, one must still ensure that mutable operations never overwrite existing files.
#[derive(Debug)]
pub struct AttachmentRoot<const EXCLUSIVE: bool> {
    lock: AttachmentRootLock<EXCLUSIVE>,
    format: AttachmentFormat,
}

pub enum AttachmentRenameOutcome {
    /// The source attachment directory was missing.
    FromMissing,
    /// The target attachment directory already exists and is non-empty.
    ToExists(PathBuf, PathBuf),
    /// The directory was renamed successfully.
    Ok,
}

impl<const EXCLUSIVE: bool> AttachmentRoot<EXCLUSIVE> {
    /// Acquire a lock (if necessary) and resolve the attachment format.
    pub fn open_or_create_unchecked(root: PathBuf, read_only: bool) -> Result<Self, anyhow::Error> {
        if !read_only {
            fs::create_dir_all(&root)?;
        }
        let mut lock = AttachmentRootLock::acquire(root, read_only)?;
        let format = lock.format()?;
        Ok(Self { lock, format })
    }

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
        let at_root = Self::open_or_create_unchecked(root, read_only)?;
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
    pub fn attachment_dir(&self, id: &Identifier) -> PathBuf {
        let mut path = PathBuf::new();
        self.attachment_dir_in(id, &mut path);
        path
    }

    /// Get the attachment directory corresponding to the identifier if the attachment directory
    /// already exists and there are existing attachments.
    ///
    /// This returns `None` if reading the directory contents fails. Typically, this happens when
    /// the attachment directory does not exist, but this also may fail for other reasons as well.
    pub fn open_attachment_dir(&self, id: &Identifier) -> Option<PathBuf> {
        let at_dir = self.attachment_dir(id);
        // TODO: is it best to iterate here, or should we just check for existence?
        if at_dir.read_dir().is_ok_and(|mut it| it.next().is_some()) {
            Some(at_dir)
        } else {
            None
        }
    }

    /// Overwrite the provided buffer with the attachment directory corresponding to the identifier.
    ///
    /// This is useful for reducing allocations in case the caller already has a [`PathBuf`].
    pub fn attachment_dir_in(&self, id: &Identifier, path: &mut PathBuf) {
        path.clear();
        path.push(self.lock.root());
        match self.format {
            AttachmentFormat::V0 => IdAttachmentPathV0(id).extend_attachments_path(path),
            AttachmentFormat::V1Migrating | AttachmentFormat::V1 => {
                IdAttachmentPathV1(id).extend_attachments_path(path);
            }
        }
    }

    /// Try to rename the attachment directory at `from` to `to`.
    ///
    /// This will not overwrite the target directory.
    pub fn rename(
        &self,
        from: &Identifier,
        to: &Identifier,
    ) -> Result<AttachmentRenameOutcome, io::Error> {
        // extra check to avoid creating the target directory unnecessarily
        // this is a toctou error, but we're doing this attachment stuff best-effort
        // anyway so it's fine
        let Some(source) = self.open_attachment_dir(from) else {
            return Ok(AttachmentRenameOutcome::FromMissing);
        };
        let target = self.attachment_dir(to);

        fs::create_dir_all(&target)?;

        match fs::rename(&source, &target) {
            Ok(()) => Ok(AttachmentRenameOutcome::Ok),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                Ok(AttachmentRenameOutcome::FromMissing)
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

struct IdAttachmentPathV0<'a, S: AsRef<str>>(&'a Identifier<S>);

struct IdAttachmentPathV1<'a, S: AsRef<str>>(&'a Identifier<S>);

pub(crate) fn extend_attachment_path_v0<S: AsRef<str>>(id: &Identifier<S>, path_buf: &mut PathBuf) {
    IdAttachmentPathV0(id).extend_attachments_path(path_buf);
}

pub(crate) fn extend_attachment_path_v1<S: AsRef<str>>(id: &Identifier<S>, path_buf: &mut PathBuf) {
    IdAttachmentPathV1(id).extend_attachments_path(path_buf);
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

impl<S: AsRef<str>> PathHash for IdAttachmentPathV0<'_, S> {
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

impl<S: AsRef<str>> PathHash for IdAttachmentPathV1<'_, S> {
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
