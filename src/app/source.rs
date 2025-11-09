use std::{
    fs::File,
    io::{Read, stdin},
};

use anyhow::bail;

use crate::{
    RecordId,
    cite_search::{SourceFileType, get_citekeys_filter},
    logger::{error, info},
};

pub fn get_citekeys_from_file<T: Extend<RecordId>, P: AsRef<std::path::Path>>(
    read_from: P,
    file_type: Option<SourceFileType>,
    container: &mut T,
    scratch: &mut Vec<u8>,
    ft_flag: &'static str,
) -> Result<(), anyhow::Error> {
    get_citekeys_from_file_filter(read_from, file_type, container, scratch, ft_flag, |_| true)
}

pub fn get_citekeys_from_stdin<T: Extend<RecordId>, E: FnMut(&RecordId) -> bool>(
    file_type: Option<SourceFileType>,
    container: &mut T,
    scratch: &mut Vec<u8>,
    exclude: E,
) -> Result<(), anyhow::Error> {
    let ft = file_type.unwrap_or(SourceFileType::Txt);
    scratch.clear();
    match stdin().read_to_end(scratch) {
        Ok(_) => {}
        Err(e) => bail!("Failed to read from standard input: '{e}'"),
    }
    get_citekeys_filter(ft, scratch, container, exclude);

    Ok(())
}

/// A wrapper around [`get_citekeys_filter`] to open the file, detect the file type (or use the provided
/// override) and then update the container with the keys.
pub fn get_citekeys_from_file_filter<
    T: Extend<RecordId>,
    P: AsRef<std::path::Path>,
    E: FnMut(&RecordId) -> bool,
>(
    read_from: P,
    file_type: Option<SourceFileType>,
    container: &mut T,
    scratch: &mut Vec<u8>,
    ft_flag: &'static str,
    exclude: E,
) -> Result<(), anyhow::Error> {
    scratch.clear();
    match File::open(&read_from).and_then(|mut f| f.read_to_end(scratch)) {
        Ok(_) => {
            if let Some(mode) = file_type.or_else(|| {
                SourceFileType::detect(&read_from).map_or_else(
                    |err| {
                        error!(
                            "File '{}': {err}. Force filetype with `{ft_flag}`.",
                            read_from.as_ref().display()
                        );
                        None
                    },
                    Some,
                )
            }) {
                info!(
                    "Reading citation keys from '{}'",
                    read_from.as_ref().display()
                );
                get_citekeys_filter(mode, scratch, container, exclude);
            }
            Ok(())
        }
        Err(err) => bail!(
            "Failed to read contents of path '{}': {err}",
            read_from.as_ref().display()
        ),
    }
}
