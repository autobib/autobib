use std::{fmt, marker::PhantomData};

use chrono::{DateTime, Local};
use crossterm::style::{ContentStyle, StyledContent, Stylize};
use ramify::TryRamify;

use super::{ArbitraryData, ArbitraryDataRef, InRecordsTable, RevisionId, State, version::Version};
use crate::{entry::EntryData, record::RemoteId};

pub struct RamifierConfig {
    pub all: bool,
    pub styled: bool,
    pub oneline: bool,
}

/// A display adapter for a record row.
#[derive(Debug)]
pub struct RecordRowDisplay<'a> {
    pub(super) data: ArbitraryDataRef<'a>,
    pub(super) modified: DateTime<Local>,
    rev_id: RevisionId,
    canonical: RemoteId<&'a str>,
    pub(super) styled: bool,
}

impl<'a> RecordRowDisplay<'a> {
    pub fn from_version(version: &'a super::Version<'_, '_>, styled: bool) -> Self {
        Self {
            data: version.row.data.get_ref(),
            modified: version.row.modified,
            rev_id: version.rev_id(),
            canonical: version.row.canonical.get_ref(),
            styled,
        }
    }

    pub fn from_borrowed_row(
        record_row: super::RecordRow<ArbitraryDataRef<'a>, &'a str>,
        rev_id: RevisionId,
        styled: bool,
    ) -> Self {
        Self {
            data: record_row.data,
            canonical: record_row.canonical,
            modified: record_row.modified,
            rev_id,
            styled,
        }
    }
}

impl<'a> fmt::Display for RecordRowDisplay<'a> {
    fn fmt(&self, buf: &mut fmt::Formatter<'_>) -> fmt::Result {
        let style = if self.styled {
            ContentStyle::default().yellow()
        } else {
            ContentStyle::default()
        };

        let hex = StyledContent::new(style, self.rev_id);

        let style = if self.styled {
            ContentStyle::default().italic().grey()
        } else {
            ContentStyle::default()
        };

        let datestamp = StyledContent::new(style, self.modified.format("on %b %d %Y at %X%Z"));

        static PREFIX: &str = "  ";

        match &self.data {
            ArbitraryDataRef::Entry(raw_entry_data) => {
                writeln!(buf, "{hex} {datestamp}\n")?;
                if self.styled {
                    writeln!(
                        buf,
                        "{PREFIX}@{}{{{},",
                        raw_entry_data.entry_type().green(),
                        self.canonical
                    )?;
                } else {
                    writeln!(
                        buf,
                        "{PREFIX}@{}{{{},",
                        raw_entry_data.entry_type(),
                        self.canonical
                    )?;
                }
                for (key, val) in raw_entry_data.fields() {
                    if self.styled {
                        writeln!(buf, "{PREFIX}  {} = {{{val}}},", key.blue())?;
                    } else {
                        writeln!(buf, "{PREFIX}  {key} = {{{val}}},",)?;
                    }
                }
                write!(buf, "{PREFIX}}}")?;

                Ok(())
            }
            ArbitraryDataRef::Deleted(replacement) => {
                writeln!(buf, "{hex} {datestamp}\n")?;
                if let Some(remote_id) = replacement {
                    write!(
                        buf,
                        "{PREFIX}Replaced '{}' with '{remote_id}'",
                        self.canonical
                    )?;
                } else {
                    write!(buf, "{PREFIX}Deleted '{}'", self.canonical)?;
                }
                Ok(())
            }
            ArbitraryDataRef::Void => {
                writeln!(buf, "{hex}\n")?;
                write!(buf, "{PREFIX}Void '{}'", self.canonical)
            }
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
            let mut children = vtx
                .children()
                .map_err(|err| ramify::Replacement { value: vtx, err })?;
            children.sort_unstable_by_key(|v| v.row.modified);
            Ok(children)
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
            RecordRowDisplay::from_version(vtx, self.config.styled),
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
            RecordRowDisplay::from_version(vtx, self.config.styled),
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
