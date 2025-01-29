use std::{
    collections::BTreeMap,
    io::{self, IsTerminal, Write},
    path::Path,
};

use itertools::Itertools;
use nonempty::NonEmpty;
use serde::Serializer as _;
use serde_bibtex::ser::Serializer;

use crate::{db::EntryData, entry::Entry, logger::warn, record::RemoteId, CitationKey};

pub fn init_outfile<P: AsRef<Path>>(
    out: Option<P>,
    append: bool,
) -> Result<Option<std::fs::File>, anyhow::Error> {
    match out.as_ref() {
        Some(path) => match std::fs::OpenOptions::new()
            .read(true)
            .create(true)
            .write(true)
            .append(append)
            .open(path)
        {
            Ok(file) => Ok(Some(file)),
            Err(e) => anyhow::bail!(
                "Failed to open output file '{}': {e}",
                path.as_ref().display()
            ),
        },
        None => Ok(None),
    }
}

pub fn output_keys<'a>(keys: impl Iterator<Item = &'a crate::RecordId>) -> Result<(), io::Error> {
    let mut stdout = io::BufWriter::new(io::stdout());
    for key in keys {
        stdout.write_all(key.name().as_bytes())?;
        stdout.write_all(b"\n")?;
    }
    Ok(())
}

/// Either write records to stdout, or to a provided file.
pub fn output_entries<D: EntryData>(
    out: Option<std::fs::File>,
    append: bool,
    grouped_entries: BTreeMap<RemoteId, NonEmpty<Entry<D>>>,
) -> Result<(), serde_bibtex::Error> {
    if let Some(file) = out {
        let mut writer = io::BufWriter::new(file);
        if append && !grouped_entries.is_empty() {
            writer.write_all(b"\n")?;
        }
        write_entries(writer, grouped_entries)?;
    } else {
        let stdout = io::stdout();
        if stdout.is_terminal() {
            // do not write an extra newline if interactive and there is nothing to write
            if !grouped_entries.is_empty() {
                write_entries(stdout, grouped_entries)?;
            }
        } else {
            let writer = io::BufWriter::new(stdout);
            write_entries(writer, grouped_entries)?;
        }
    };

    Ok(())
}

/// Iterate over records, writing the entries and warning about duplicates.
fn write_entries<W: io::Write, D: EntryData>(
    writer: W,
    grouped_entries: BTreeMap<RemoteId, NonEmpty<Entry<D>>>,
) -> Result<(), serde_bibtex::Error> {
    let mut serializer = Serializer::unchecked(writer);

    serializer.collect_seq(grouped_entries.iter().flat_map(|(canonical, entry_group)| {
        if entry_group.len() > 1 {
            warn!(
                "Multiple keys for '{canonical}': {}",
                entry_group.iter().map(|e| e.key().as_ref()).join(", ")
            );
        };
        entry_group
    }))
}
