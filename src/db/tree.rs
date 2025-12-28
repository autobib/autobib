use std::{fmt::Write as _, marker::PhantomData};

use crossterm::style::{ContentStyle, StyledContent, Stylize};
use ramify::TryRamify;

use crate::db::state::ArbitraryData;

use super::state::{InRecordsTable, RecordRowDisplay, State, Version};

pub struct RamifierConfig {
    pub all: bool,
    pub styled: bool,
    pub oneline: bool,
}

/// A ramifier designed for version history.
pub struct FullHistoryRamifier<'tx> {
    active_row_id: i64,
    config: RamifierConfig,
    _marker: PhantomData<&'tx ()>,
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

impl<'tx, 'conn> TryRamify<Version<'tx, 'conn>> for FullHistoryRamifier<'tx> {
    type Error = rusqlite::Error;

    fn try_ramify(
        &mut self,
        vtx: Version<'tx, 'conn>,
    ) -> Result<impl IntoIterator<Item = Version<'tx, 'conn>>, Self::Error> {
        // we always iterate over children if we are on an entry; otherwise, only iterate if 'all'
        if vtx.is_entry() || self.config.all {
            let mut children = vtx.children()?;
            children.sort_unstable_by_key(|v| v.row.modified);
            Ok(children)
        } else {
            Ok(Vec::new())
        }
    }

    fn sort_key(&self, vtx: &Version<'tx, 'conn>) -> impl Ord {
        &vtx.row.modified
    }

    fn marker(&self, vtx: &Version<'tx, 'conn>) -> char {
        vtx.marker(self.active_row_id)
    }

    fn annotate(&self, vtx: &Version<'tx, 'conn>, buf: &mut String) {
        let disp = StyledContent::new(
            ContentStyle::default(),
            RecordRowDisplay::from_version(vtx, self.config.styled),
        );

        let disp = if self.config.styled && vtx.row_id == self.active_row_id {
            disp.bold()
        } else {
            disp
        };

        write!(buf, "{disp}").expect("Writing to a string should not fail");
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

    fn try_ramify(
        &mut self,
        vtx: Version<'tx, 'conn>,
    ) -> Result<impl IntoIterator<Item = Version<'tx, 'conn>>, Self::Error> {
        match vtx.parent()? {
            None => Ok(None),
            Some(parent) => {
                // since this method iterates backwards, we perform the check on the next version
                // and only yield it if it is an entry
                if parent.is_entry() || self.config.all {
                    Ok(Some(parent))
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn sort_key(&self, vtx: &Version<'tx, 'conn>) -> impl Ord {
        &vtx.row.modified
    }

    fn marker(&self, vtx: &Version<'tx, 'conn>) -> char {
        vtx.marker(self.active_row_id)
    }

    fn annotate(&self, vtx: &Version<'tx, 'conn>, buf: &mut String) {
        let disp = StyledContent::new(
            ContentStyle::default(),
            RecordRowDisplay::from_version(vtx, self.config.styled),
        );

        let disp = if self.config.styled && vtx.row_id == self.active_row_id {
            disp.bold()
        } else {
            disp
        };

        write!(buf, "{disp}").expect("Writing to a string should not fail");
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
