use std::{fmt, marker::PhantomData};

use crossterm::style::{ContentStyle, StyledContent, Stylize};
use ramify::TryRamify;

use super::{ArbitraryData, InRecordsTable, State, version::Version};
use crate::entry::EntryData;

struct LogEntry<'a, 'tx, 'conn> {
    version: &'a Version<'tx, 'conn>,
    styled: bool,
}

impl<'a, 'tx, 'conn> fmt::Display for LogEntry<'a, 'tx, 'conn> {
    fn fmt(&self, buf: &mut fmt::Formatter<'_>) -> fmt::Result {
        let style = if self.styled {
            ContentStyle::default().yellow()
        } else {
            ContentStyle::default()
        };

        let hex = StyledContent::new(style, self.version.rev_id());

        match &self.version.row.data {
            ArbitraryData::Entry(raw_entry_data) => {
                writeln!(buf, "  @{}{{{hex},", raw_entry_data.entry_type())?;
                for (key, val) in raw_entry_data.fields() {
                    writeln!(buf, "    {key} = {{{val}}},")?;
                }
                writeln!(buf, "  }}")?;

                Ok(())
            }
            ArbitraryData::Deleted(Some(remote_id)) => {
                write!(buf, "{hex} Replaced by '{remote_id}'")
            }
            ArbitraryData::Deleted(None) => write!(buf, "{hex} Deleted"),
            ArbitraryData::Void => write!(buf, "{hex} Voided"),
        }
    }
}

impl<'tx, 'conn> Version<'tx, 'conn> {
    fn marker(&self, row_id: i64) -> char {
        match self.row.data {
            ArbitraryData::Entry(_) => {
                if self.row_id == row_id {
                    '◉'
                } else {
                    '○'
                }
            }
            ArbitraryData::Deleted(_) => {
                if self.row_id == row_id {
                    '⊗'
                } else {
                    '✕'
                }
            }
            ArbitraryData::Void => '∅',
        }
    }
}

/// A ramifier designed for version history.
pub struct FullHistoryRamifier<'tx>(i64, PhantomData<&'tx ()>);

impl<'tx, 'conn> TryRamify<Version<'tx, 'conn>> for FullHistoryRamifier<'tx> {
    type Error = rusqlite::Error;

    fn try_children(
        &mut self,
        vtx: Version<'tx, 'conn>,
    ) -> Result<
        impl IntoIterator<Item = Version<'tx, 'conn>>,
        ramify::Replacement<Version<'tx, 'conn>, Self::Error>,
    > {
        vtx.child_iter()
            .rev()
            .collect::<rusqlite::Result<Vec<Version<'tx, 'conn>>>>()
            .map_err(|err| ramify::Replacement { value: vtx, err })
    }

    fn get_key(&self, vtx: &Version<'tx, 'conn>) -> impl Ord {
        &vtx.row.modified
    }

    fn marker(&self, vtx: &Version<'tx, 'conn>) -> char {
        vtx.marker(self.0)
    }

    fn annotation<B: fmt::Write>(&self, vtx: &Version<'tx, 'conn>, mut buf: B) -> fmt::Result {
        let disp = StyledContent::new(
            ContentStyle::default(),
            LogEntry {
                version: vtx,
                styled: true,
            },
        );

        if vtx.row_id == self.0 {
            write!(buf, "{}", disp.bold())
        } else {
            write!(buf, "{disp}")
        }
    }
}

/// A ramifier which iterates over the immediate history.
pub struct AncestorRamifier<'tx>(i64, PhantomData<&'tx ()>);

impl<'tx, 'conn> TryRamify<Version<'tx, 'conn>> for AncestorRamifier<'tx> {
    type Error = rusqlite::Error;

    fn try_children(
        &mut self,
        vtx: Version<'tx, 'conn>,
    ) -> Result<
        impl IntoIterator<Item = Version<'tx, 'conn>>,
        ramify::Replacement<Version<'tx, 'conn>, Self::Error>,
    > {
        match vtx
            .parent()
            .map_err(|err| ramify::Replacement { value: vtx, err })?
        {
            None => Ok(None.into_iter()),
            Some(parent) => Ok(Some(parent).into_iter()),
        }
    }

    fn get_key(&self, vtx: &Version<'tx, 'conn>) -> impl Ord {
        &vtx.row.modified
    }

    fn marker(&self, vtx: &Version<'tx, 'conn>) -> char {
        vtx.marker(self.0)
    }

    fn annotation<B: fmt::Write>(&self, vtx: &Version<'tx, 'conn>, mut buf: B) -> fmt::Result {
        let disp = StyledContent::new(
            ContentStyle::default(),
            LogEntry {
                version: vtx,
                styled: true,
            },
        );

        if vtx.row_id == self.0 {
            write!(buf, "{}", disp.bold())
        } else {
            write!(buf, "{disp}")
        }
    }
}

/// Changelog implementation
impl<'conn, I: InRecordsTable> State<'conn, I> {
    pub fn ancestor_ramifier(&self) -> AncestorRamifier<'_> {
        AncestorRamifier(self.row_id(), PhantomData)
    }

    pub fn full_history_ramifier(&self) -> FullHistoryRamifier<'_> {
        FullHistoryRamifier(self.row_id(), PhantomData)
    }
}
