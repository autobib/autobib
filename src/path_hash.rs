use std::{hash::Hasher, path::PathBuf, str::from_utf8_unchecked};

use data_encoding::BASE32;
use rapidhash::RapidInlineHasher;

use crate::RemoteId;

/// A type which can be encoded as a platform-friendly path into a buffer.
pub trait PathHash {
    /// Extend the provided buffer with a hashed version of the path.
    fn path_hash(&self, path_buf: &mut PathBuf);
}

impl PathHash for RemoteId {
    /// In order to reduce the number of files which are in the same directory, we apply a 30-bit
    /// header to each path, which is encoded in base32 as `xx/xx/xx`. Then the corresponding path
    /// is
    /// ```text
    /// provider/xx/xx/xx/base32-encoding-of-sub-id/
    /// ```
    /// The 30 bit header is formed by converting the u64 output of the [rapidhash] hashing
    /// algorithm applied to the sub-id into little endian bytes, then taking the four most
    /// significant bytes (in decreasing order), encoding using BASE32 into 8 ASCII characters,
    /// and then taking the first 6.
    ///
    /// The header `xx/xx/xx` ensures that each directory does not have more than 1024 immediate
    /// sub-directories.
    fn path_hash(&self, path_buf: &mut PathBuf) {
        let sub_id_bytes = self.sub_id().as_bytes();
        let mut hasher = RapidInlineHasher::default();
        hasher.write(sub_id_bytes);
        let sub_id_hash: [u8; 8] = hasher.finish().to_le_bytes();

        let mut buffer = [0; 8];
        BASE32.encode_mut(&sub_id_hash[..4], &mut buffer);
        let sub_id_encoded: String = BASE32.encode(sub_id_bytes);
        path_buf.extend([
            self.provider(),
            // SAFETY: BASE32 encoding only returns ASCII bytes
            unsafe { from_utf8_unchecked(&buffer[0..2]) },
            unsafe { from_utf8_unchecked(&buffer[2..4]) },
            unsafe { from_utf8_unchecked(&buffer[4..6]) },
            &sub_id_encoded,
            // This appends a `/` or `\`, as platform appropriate, to be clear that this
            // is a directory
            "",
        ]);
    }
}
