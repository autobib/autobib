use std::io::Write;

use anyhow::Result;
use nucleo_picker::Selection;

use crate::{db::state::Record, entry::RawEntryData, logger::error, output::stdout_lock_wrap};

pub trait FindSelection<'a> {
    fn is_empty(&self) -> bool;
    fn iter(&'a self) -> impl Iterator<Item = &'a Record<RawEntryData>>;
}

impl<'a> FindSelection<'a> for Option<&'a Record<RawEntryData>> {
    fn is_empty(&self) -> bool {
        self.is_none()
    }

    fn iter(&'a self) -> impl Iterator<Item = &'a Record<RawEntryData>> {
        (*self).into_iter()
    }
}

impl<'a> FindSelection<'a> for Selection<'a, Record<RawEntryData>> {
    fn is_empty(&self) -> bool {
        Selection::is_empty(self)
    }

    fn iter(&'a self) -> impl Iterator<Item = &'a Record<RawEntryData>> {
        Selection::iter(self)
    }
}

pub fn output_find_selection<'a>(selection: &'a impl FindSelection<'a>) -> Result<()> {
    if selection.is_empty() {
        error!("No item selected.");
    } else {
        let mut stdout = stdout_lock_wrap();
        for row_data in selection.iter() {
            writeln!(&mut stdout, "{}", row_data.canonical)?;
        }
    }

    Ok(())
}
