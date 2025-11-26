use std::{fmt, marker::PhantomData};

use crossterm::style::{ContentStyle, StyledContent, Stylize};
use ramify::TryRamify;

use super::{ArbitraryData, InRecordsTable, State, version::Version};
use crate::entry::EntryData;

pub struct RamifierConfig {
    pub all: bool,
    pub styled: bool,
    pub oneline: bool,
}

#[derive(Debug)]
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
                write!(buf, "  }}")?;

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
pub struct FullHistoryRamifier<'tx> {
    active_row_id: i64,
    config: RamifierConfig,
    _marker: PhantomData<&'tx ()>,
}

impl<'tx, 'conn> TryRamify<Version<'tx, 'conn>> for FullHistoryRamifier<'tx> {
    type Error = rusqlite::Error;

    fn try_children(
        &mut self,
        vtx: Version<'tx, 'conn>,
    ) -> Result<
        impl IntoIterator<Item = Version<'tx, 'conn>>,
        ramify::Replacement<Version<'tx, 'conn>, Self::Error>,
    > {
        // we always iterate over children if we are on an entry; otherwise, only iterate if 'all'
        if vtx.is_entry() || self.config.all {
            vtx.child_iter()
                .rev()
                .collect::<rusqlite::Result<Vec<Version<'tx, 'conn>>>>()
                .map_err(|err| ramify::Replacement { value: vtx, err })
        } else {
            Ok(Vec::new())
        }
    }

    fn get_key(&self, vtx: &Version<'tx, 'conn>) -> impl Ord {
        &vtx.row.modified
    }

    fn marker(&self, vtx: &Version<'tx, 'conn>) -> char {
        vtx.marker(self.active_row_id)
    }

    fn annotation<B: fmt::Write>(&self, vtx: &Version<'tx, 'conn>, mut buf: B) -> fmt::Result {
        let disp = StyledContent::new(
            ContentStyle::default(),
            LogEntry {
                version: vtx,
                styled: self.config.styled,
            },
        );

        let disp = if self.config.styled && vtx.row_id == self.active_row_id {
            disp.bold()
        } else {
            disp
        };

        write!(buf, "{disp}")
    }
}

/// A ramifier which iterates over the immediate history.
pub struct AncestorRamifier<'tx> {
    active_row_id: i64,
    config: RamifierConfig,
    _marker: PhantomData<&'tx ()>,
}

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
            Some(parent) => {
                // since this method iterates backwards, we perform the check on the next version
                // and only yield it if it is an entry
                if parent.is_entry() || self.config.all {
                    Ok(Some(parent).into_iter())
                } else {
                    Ok(None.into_iter())
                }
            }
        }
    }

    fn get_key(&self, vtx: &Version<'tx, 'conn>) -> impl Ord {
        &vtx.row.modified
    }

    fn marker(&self, vtx: &Version<'tx, 'conn>) -> char {
        vtx.marker(self.active_row_id)
    }

    fn annotation<B: fmt::Write>(&self, vtx: &Version<'tx, 'conn>, mut buf: B) -> fmt::Result {
        let disp = StyledContent::new(
            ContentStyle::default(),
            LogEntry {
                version: vtx,
                styled: self.config.styled,
            },
        );

        let disp = if self.config.styled && vtx.row_id == self.active_row_id {
            disp.bold()
        } else {
            disp
        };

        write!(buf, "{disp}")
    }
}

/// Changelog implementation
impl<'conn, I: InRecordsTable> State<'conn, I> {
    pub fn ancestor_ramifier(&self, config: RamifierConfig) -> AncestorRamifier<'_> {
        AncestorRamifier {
            active_row_id: self.row_id(),
            config,
            _marker: PhantomData,
        }
    }

    pub fn full_history_ramifier(&self, config: RamifierConfig) -> FullHistoryRamifier<'_> {
        FullHistoryRamifier {
            active_row_id: self.row_id(),
            config,
            _marker: PhantomData,
        }
    }
}
