//! # Extract citation keys from files
//! This module provides convenient functionality for extracting citation keys from arbitrary
//! bytes, such as the contents of files.
//!
//! ## Example
//! ```
//! use autobib::citekey::{SourceFileType, get_citekeys};
//! use std::iter::zip;
//! use std::collections::BTreeSet;
//!
//! let contents = r"
//!     An explanation can be found in \cite[§2]{ref2} (see also \cite{ref1,
//!     ref3})."
//!     .as_bytes();
//!
//! let mut container = BTreeSet::new();
//!
//! get_citekeys(SourceFileType::Tex, contents, &mut container);
//!
//! let expected = vec!["ref1", "ref2", "ref3"];
//! for (exp, rec) in zip(expected.iter(), container.iter()) {
//!     assert_eq!(exp, rec);
//! }
//! ```
pub mod tex;

use std::ffi::OsStr;
use std::iter::Extend;
use std::path::Path;
use std::str::FromStr;

use crate::error::Error;

/// The file type of a source from which citation keys can be read.
#[derive(Debug, Clone, Copy)]
pub enum SourceFileType {
    /// TeX-style contents, such as `.tex` or `.sty` files.
    Tex,
}

impl FromStr for SourceFileType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tex" | "sty" | "cls" => Ok(Self::Tex),
            ext => Err(Error::UnsupportedFileType(ext.into())),
        }
    }
}

impl SourceFileType {
    /// Detect the file type automatically from the provided path.
    ///
    /// If the file type is not supported, or detection fails, this returns an error.
    pub fn detect<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        match path.as_ref().extension().and_then(OsStr::to_str) {
            Some(ext) => Self::from_str(ext),
            None => Err(Error::MissingFileType),
        }
    }
}

/// Read citekeys from a byte buffer into a container.
///
/// The byte buffer is assumed to have file type specified by `ft`.
/// The citekeys are inserted into the container using the container's [`Extend`] implementation.
/// The order is is not necessarily the same as the order of the keys in the buffer.
pub fn get_citekeys<T: Extend<String>>(ft: SourceFileType, buffer: &[u8], container: &mut T) {
    match ft {
        SourceFileType::Tex => tex::get_citekeys(buffer, container),
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeSet;
    use std::iter::zip;

    use super::*;

    #[test]
    fn test_get_citekeys_tex() {
        let contents = r"
            An explanation can be found in \cite[§2]{ref2} (see also \cite{ref1,
            ref3})."
            .as_bytes();

        let mut container = BTreeSet::new();

        get_citekeys(SourceFileType::Tex, contents, &mut container);

        let expected = ["ref1", "ref2", "ref3"];
        for (exp, rec) in zip(expected.iter(), container.iter()) {
            assert_eq!(exp, rec);
        }
    }
}
