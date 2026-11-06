//! # Display adapter for a record row
//!
//! The only struct here is [`RecordRowDisplay`], which is used to display the data present in a
//! row in the 'Records' table.
use std::fmt;

use autobib_entry::data::EntryData;
use crossterm::style::{ContentStyle, StyledContent, Stylize};

use super::{ArbitraryDataRef, InRecordsTable, Record, State, Version, WithRev};
use crate::{db::AsKey, logger::LogDisplay};

pub(crate) fn render_bibtex(
    data: &impl EntryData,
    canonical: &str,
    prefix: &str,
    mut write: impl FnMut(&str, ContentStyle) -> fmt::Result,
) -> fmt::Result {
    let plain = ContentStyle::default();
    write(prefix, plain)?;
    write("@", plain.cyan())?;
    write(data.entry_type().inner(), plain.green())?;
    write("{", plain.cyan())?;
    write(canonical, plain)?;
    write(",\n", plain)?;
    for (key, value) in data.fields() {
        write(prefix, plain)?;
        write("  ", plain)?;
        write(key.inner(), plain.blue())?;
        write(" = ", plain)?;
        write("{", plain.cyan())?;
        write(value.inner(), plain)?;
        write("}", plain.cyan())?;
        write(",\n", plain)?;
    }
    write(prefix, plain)?;
    write("}", plain.cyan())
}

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
    pub(super) tagged: WithRev<Record<ArbitraryDataRef<'a>, &'a str>>,
}

impl<'a> RecordRowDisplay<'a> {
    /// Construct this display adapter by borrowing data from a [`Version`].
    pub fn from_version(version: &'a Version<'_, '_>, styled: bool) -> Self {
        let inner = Record {
            data: version.hist.record.data.as_deref(),
            modified: version.hist.record.modified,
            canonical: version.hist.record.canonical.as_deref(),
        };

        let tagged = WithRev {
            rev: version.rev_id(),
            inner,
        };
        Self { tagged, styled }
    }

    /// Construct this display adapter by borrowing data the components of a row.
    pub fn from_borrowed_row(
        tagged: WithRev<Record<ArbitraryDataRef<'a>, &'a str>>,
        styled: bool,
    ) -> Self {
        Self { tagged, styled }
    }
}

impl<'a> fmt::Display for RecordRowDisplay<'a> {
    fn fmt(&self, buf: &mut fmt::Formatter<'_>) -> fmt::Result {
        let style = if self.styled {
            ContentStyle::default().yellow()
        } else {
            ContentStyle::default()
        };

        let hex = StyledContent::new(style, self.tagged.rev.fmt_pretty());

        let style = if self.styled {
            ContentStyle::default().italic().grey()
        } else {
            ContentStyle::default()
        };

        let datestamp = StyledContent::new(
            style,
            self.tagged.inner.modified.format("on %b %d %Y at %X%Z"),
        );

        static PREFIX: &str = "  ";
        let canonical = &self.tagged.inner.canonical;
        match &self.tagged.inner.data {
            ArbitraryDataRef::Entry(raw_entry_data) => {
                writeln!(buf, "{hex} {datestamp}\n")?;
                render_bibtex(raw_entry_data, canonical.as_key(), PREFIX, |text, style| {
                    if self.styled {
                        write!(buf, "{}", style.apply(text))
                    } else {
                        buf.write_str(text)
                    }
                })
            }
            ArbitraryDataRef::Deleted(replacement) => {
                writeln!(buf, "{hex} {datestamp}\n")?;
                if let Some(id) = replacement {
                    write!(buf, "{PREFIX}Replaced '{canonical}' with '{id}'")?;
                } else {
                    write!(buf, "{PREFIX}Deleted '{canonical}'")?;
                }
                Ok(())
            }
            ArbitraryDataRef::Void => {
                writeln!(buf, "{hex}\n")?;
                write!(buf, "{PREFIX}Void '{canonical}'")
            }
        }
    }
}
