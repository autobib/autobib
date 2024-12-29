use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    path::Path,
};

use itertools::Itertools;
use nonempty::NonEmpty;
use serde::Serializer as _;
use serde_bibtex::ser::Serializer;

use crate::{db::EntryData, entry::Entry, logger::warn, record::RemoteId};

/// Either write records to stdout, or to a provided file.
pub fn output_entries<D: EntryData, P: AsRef<Path>>(
    out: Option<P>,
    grouped_entries: BTreeMap<RemoteId, NonEmpty<Entry<D>>>,
) -> Result<(), serde_bibtex::Error> {
    if let Some(path) = out {
        let writer = io::BufWriter::new(std::fs::File::create(path)?);
        write_entries(writer, grouped_entries)?;
    } else {
        let stdout = io::stdout();
        if stdout.is_terminal() {
            // do not write an extra newline if interactive
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
