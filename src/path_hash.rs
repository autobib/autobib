use std::path::PathBuf;

use data_encoding::BASE32;
use rapidhash::v1::rapidhash_v1;

use crate::RemoteId;

/// A type which can be encoded as a platform-friendly path into a buffer.
pub trait PathHash {
    /// Extend the provided buffer with a hashed version of the path.
    fn extend_attachments_path(&self, path_buf: &mut PathBuf);
}

impl PathHash for RemoteId {
    /// In order to reduce the number of files which are in the same directory, we apply a 30-bit
    /// header to each path, which is encoded in base32 as `xx/xx/xx`. Then the corresponding path
    /// is
    /// ```text
    /// provider/xx/xx/xx/base32-encoding-of-sub-id/
    /// ```
    /// The 30 bit header is formed by converting the u64 output of the [rapidhash::v1] hashing
    /// algorithm applied to the sub-id into little endian bytes, then taking the four most
    /// significant bytes (in decreasing order), encoding using BASE32 into 8 ASCII characters,
    /// and then taking the first 6.
    ///
    /// The header `xx/xx/xx` ensures that each directory does not have more than 1024 immediate
    /// sub-directories.
    fn extend_attachments_path(&self, path_buf: &mut PathBuf) {
        let sub_id_bytes = self.sub_id().as_bytes();
        let sub_id_hash: [u8; 8] = rapidhash_v1(sub_id_bytes).to_le_bytes();

        let mut buffer = [0; 8];
        let res = BASE32.encode_mut_str(&sub_id_hash[..4], &mut buffer);
        let sub_id_encoded: String = BASE32.encode(sub_id_bytes);
        path_buf.extend([
            self.provider(),
            &res[0..2],
            &res[2..4],
            &res[4..6],
            &sub_id_encoded,
        ]);
    }
}
