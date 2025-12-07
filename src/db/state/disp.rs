//! # Display adapter for a record row
//!
//! The only struct here is [`RecordRowDisplay`], which is used to display the data present in a
//! row in the 'Records' table.
use std::fmt;

use chrono::{DateTime, Local};
use crossterm::style::{ContentStyle, StyledContent, Stylize};

use super::{ArbitraryDataRef, InRecordsTable, RecordRow, RevisionId, State, Version};
use crate::{entry::EntryData, logger::LogDisplay, record::RemoteId};

impl<'conn, I: InRecordsTable> LogDisplay for State<'conn, I> {
    fn log_display(&self, styled: bool, mut buf: impl std::io::Write) -> anyhow::Result<()> {
        writeln!(buf, "{}", self.current()?.display(styled))?;
        Ok(())
    }
}

/// A display adapter for a row in the 'Records' table.
#[derive(Debug)]
pub struct RecordRowDisplay<'a> {
    /// Whether or not the display should be 'styled' (using colours, bold, etc.)
    pub styled: bool,
    pub(super) data: ArbitraryDataRef<'a>,
    pub(super) modified: DateTime<Local>,
    rev_id: RevisionId,
    canonical: RemoteId<&'a str>,
}

impl<'a> RecordRowDisplay<'a> {
    /// Construct this display adapter by borrowing data from a [`Version`].
    pub fn from_version(version: &'a Version<'_, '_>, styled: bool) -> Self {
        Self {
            data: version.row.data.get_ref(),
            modified: version.row.modified,
            rev_id: version.rev_id(),
            canonical: version.row.canonical.get_ref(),
            styled,
        }
    }

    /// Construct this display adapter by borrowing data the components of a row.
    pub fn from_borrowed_row(
        record_row: RecordRow<ArbitraryDataRef<'a>, &'a str>,
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
