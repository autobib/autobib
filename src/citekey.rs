pub mod tex;

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;

use crate::error::Error;

#[derive(Debug, Clone, Copy)]
pub enum SourceFileType {
    Tex,
}

impl std::str::FromStr for SourceFileType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tex" => Ok(Self::Tex),
            ext => Err(Error::UnsupportedFileType(ext.into())),
        }
    }
}

/// Detect the citekey mode automatically based on the provided path.
pub fn guess_source_file_type(path: &Path) -> Result<SourceFileType, Error> {
    match path.extension().and_then(OsStr::to_str) {
        Some("tex" | "sty" | "cls") => Ok(SourceFileType::Tex),
        Some(ext) => Err(Error::UnsupportedFileType(ext.into())),
        None => Err(Error::MissingFileType),
    }
}

pub fn get_citekeys(mode: SourceFileType, buffer: &[u8], keys: &mut HashSet<String>) {
    match mode {
        SourceFileType::Tex => tex::get_citekeys(buffer, keys),
    }
}
