use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{self, IsTerminal, Write},
    path::Path,
};

use itertools::Itertools;
use nonempty::NonEmpty;
use serde::{Serialize, ser::SerializeMap};

use crate::{
    AsKey,
    entry::{AsEntryData, BibtexEntry, EntryData, entries_to_bibtex},
    logger::warn,
    output::stdout_lock_wrap,
    record::Identifier,
};

pub fn init_outfile<P: AsRef<Path>>(
    out: Option<P>,
    append: bool,
) -> Result<Option<std::fs::File>, anyhow::Error> {
    match out.as_ref() {
        Some(path) => {
            let mut opts = OpenOptions::new();
            opts.create(true);

            if append {
                opts.read(true).append(true);
            } else {
                opts.write(true).truncate(true);
            }

            match opts.open(path) {
                Ok(file) => Ok(Some(file)),
                Err(e) => anyhow::bail!(
                    "Failed to open output file '{}': {e}",
                    path.as_ref().display()
                ),
            }
        }
        None => Ok(None),
    }
}

pub fn output_keys<'a>(keys: impl Iterator<Item = &'a crate::Key>) -> Result<(), io::Error> {
    let mut stdout = io::BufWriter::new(stdout_lock_wrap());
    for key in keys {
        stdout.write_all(key.as_key().as_bytes())?;
        stdout.write_all(b"\n")?;
    }
    Ok(())
}

pub fn output_entries_json<D: AsEntryData>(
    grouped_entries: &BTreeMap<Identifier, NonEmpty<BibtexEntry<D>>>,
) -> Result<(), anyhow::Error> {
    struct Wrapper<'a, D>(&'a BTreeMap<Identifier, NonEmpty<BibtexEntry<D>>>);

    impl<'a, D: AsEntryData> Serialize for Wrapper<'a, D> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut state = serializer.serialize_map(None)?;
            for entry in flatten_and_warn(self.0) {
                state.serialize_entry(
                    entry.key.as_ref(),
                    &entry.record_data.as_entry_data().serialize(),
                )?;
            }
            state.end()
        }
    }

    let mut lock = stdout_lock_wrap();
    serde_json::to_writer_pretty(&mut lock, &Wrapper(grouped_entries))?;
    writeln!(lock)?;
    Ok(())
}

/// Either write records to stdout, or to a provided file.
pub fn output_entries_bibtex<D: AsEntryData>(
    out: Option<std::fs::File>,
    append: bool,
    grouped_entries: &BTreeMap<Identifier, NonEmpty<BibtexEntry<D>>>,
) -> Result<(), serde_bibtex::Error> {
    match out {
        Some(file) => {
            let mut writer = io::BufWriter::new(file);
            if append && !grouped_entries.is_empty() {
                writer.write_all(b"\n")?;
            }
            entries_to_bibtex(writer, flatten_and_warn(grouped_entries))?;
        }
        _ => {
            let stdout = io::stdout();
            if stdout.is_terminal() {
                // do not write an extra newline if interactive and there is nothing to write
                if !grouped_entries.is_empty() {
                    // no need to use `stdout_lock_wrap` as broken pipe error cannot occur
                    entries_to_bibtex(stdout.lock(), flatten_and_warn(grouped_entries))?;
                }
            } else {
                let writer = io::BufWriter::new(stdout_lock_wrap());
                entries_to_bibtex(writer, flatten_and_warn(grouped_entries))?;
            }
        }
    };

    Ok(())
}

fn flatten_and_warn<D>(
    grouped_entries: &BTreeMap<Identifier, NonEmpty<BibtexEntry<D>>>,
) -> impl Iterator<Item = &BibtexEntry<D>> {
    grouped_entries.iter().flat_map(|(canonical, entry_group)| {
        if entry_group.len() > 1 {
            warn!(
                "Multiple keys for '{canonical}': {}",
                entry_group.iter().map(|e| e.key().as_ref()).join(", ")
            );
        };
        entry_group
    })
}
