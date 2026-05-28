use std::{io::Write, thread};

use anyhow::Result;
use nucleo_picker::Selection;

use crate::{
    db::{RecordDatabase, state::RecordRow},
    entry::RawEntryData,
    logger::error,
    output::stdout_lock_wrap,
};

pub trait FindSelection<'a> {
    fn is_empty(&self) -> bool;
    fn iter(&'a self) -> impl Iterator<Item = &'a RecordRow<RawEntryData>>;
}

impl<'a> FindSelection<'a> for Option<&'a RecordRow<RawEntryData>> {
    fn is_empty(&self) -> bool {
        self.is_none()
    }

    fn iter(&'a self) -> impl Iterator<Item = &'a RecordRow<RawEntryData>> {
        (*self).into_iter()
    }
}

impl<'a> FindSelection<'a> for Selection<'a, RecordRow<RawEntryData>> {
    fn is_empty(&self) -> bool {
        Selection::is_empty(self)
    }

    fn iter(&'a self) -> impl Iterator<Item = &'a RecordRow<RawEntryData>> {
        Selection::iter(self)
    }
}

pub fn output_find_selection<'a>(
    selection: &'a impl FindSelection<'a>,
    canonical: bool,
    preferred_providers: &[String],
    handle: thread::JoinHandle<Result<RecordDatabase, rusqlite::Error>>,
) -> Result<()> {
    if selection.is_empty() {
        error!("No item selected.");
    } else {
        let mut stdout = stdout_lock_wrap();
        if canonical || preferred_providers.is_empty() {
            // don't bother to wait for the thread to join
            for row_data in selection.iter() {
                writeln!(&mut stdout, "{}", row_data.canonical)?;
            }
        } else {
            let mut record_db = handle.join().expect("Thread should not have panicked")?;
            let snapshot = record_db.snapshot()?;
            let mut referencing_ids = Vec::new();

            if preferred_providers.len() <= 4 {
                // don't bother sorting
                's: for row_data in selection.iter() {
                    referencing_ids.clear();
                    snapshot.equivalent_remote_ids(&row_data.canonical, |id| {
                        referencing_ids.push(id);
                    })?;

                    for provider in preferred_providers {
                        if let Some(remote_id) =
                            referencing_ids.iter().find(|id| id.provider() == provider)
                        {
                            writeln!(&mut stdout, "{remote_id}")?;
                            continue 's;
                        }
                    }
                    // no matching preferred provider
                    writeln!(&mut stdout, "{}", row_data.canonical)?;
                }
            } else {
                // get a key from the preferred provider if possible
                's: for row_data in selection.iter() {
                    referencing_ids.clear();
                    snapshot.equivalent_remote_ids(&row_data.canonical, |id| {
                        referencing_ids.push(id);
                    })?;

                    referencing_ids.sort_unstable_by(|l, r| l.provider().cmp(r.provider()));

                    for provider in preferred_providers {
                        if let Ok(idx) =
                            referencing_ids.binary_search_by(|id| id.provider().cmp(provider))
                        {
                            writeln!(&mut stdout, "{}", &referencing_ids[idx])?;
                            continue 's;
                        }
                    }
                    // no matching preferred provider
                    writeln!(&mut stdout, "{}", row_data.canonical)?;
                }
            }
        }
    }

    Ok(())
}
