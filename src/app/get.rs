use std::{
    convert::Infallible,
    io::IsTerminal,
    io::{self, BufRead},
};

use autobib_entry::v1::ArchivedEntryData;

use crate::{
    app::retrieve::{self, retrieve_single_entry, retrieve_single_entry_read_only},
    config::Config,
    db::{
        RecordDatabase,
        select::{MapRow, col},
        state::{IsEntry, Record, State},
    },
    entry::BibtexEntry,
    error::DatabaseError,
    format::{Template, TemplateData},
    http::Client,
    record::{Identifier, Key, KeyedRecord},
};

/// Types which can be obtained from a [`KeyedRecord`] after potentially retrieving additional data the
/// provided entry row.
pub trait TryFromDbState: Sized {
    fn filter_map(
        record: KeyedRecord,
        row: &State<'_, IsEntry>,
    ) -> Result<Option<Self>, DatabaseError>;
}

impl TryFromDbState for Infallible {
    fn filter_map(_: KeyedRecord, _: &State<'_, IsEntry>) -> Result<Option<Self>, DatabaseError> {
        Ok(None)
    }
}

impl TryFromDbState for KeyedRecord {
    fn filter_map(
        record: KeyedRecord,
        _: &State<'_, IsEntry>,
    ) -> Result<Option<Self>, DatabaseError> {
        Ok(Some(record))
    }
}

impl TryFromDbState for (BibtexEntry, Identifier) {
    fn filter_map(
        record: KeyedRecord,
        row: &State<'_, IsEntry>,
    ) -> Result<Option<Self>, DatabaseError> {
        Ok(retrieve::try_data_to_entry(record, row))
    }
}

/// Writers which know how to write certain items into an IO stream.
pub trait Output<D: ?Sized> {
    /// Write a single item.
    fn write_item(&mut self, item: &D) -> Result<(), io::Error>;

    /// Called after all of the items have been written.
    fn finish(self) -> Result<(), io::Error>;
}

/// A retriever which has an associated data type which can be read from a database record and
/// state, and which can output that data.
pub trait Get: Output<Self::Data> {
    type Data: TryFromDbState + ?Sized;
}

pub struct NoOutput;

impl Output<Infallible> for NoOutput {
    fn write_item(&mut self, data: &Infallible) -> Result<(), io::Error> {
        match *data {}
    }

    fn finish(self) -> Result<(), io::Error> {
        Ok(())
    }
}

impl Get for NoOutput {
    type Data = Infallible;
}

pub struct BibtexOutput<'r, W: ?Sized> {
    first: bool,
    writer: &'r mut W,
}

impl<'r, W: io::Write + ?Sized> BibtexOutput<'r, W> {
    pub fn new(writer: &'r mut W) -> Self {
        Self {
            first: true,
            writer,
        }
    }
}

impl<'r, W: io::Write + ?Sized> Output<(BibtexEntry, Identifier)> for BibtexOutput<'r, W> {
    fn write_item(&mut self, (entry, _): &(BibtexEntry, Identifier)) -> Result<(), io::Error> {
        if self.first {
            self.first = false;
        } else {
            write!(self.writer, "\n\n")?;
        }

        entry.write_io(&mut self.writer)
    }

    fn finish(self) -> Result<(), io::Error> {
        if self.first {
            Ok(())
        } else {
            writeln!(self.writer)
        }
    }
}

impl<'r, W: io::Write + ?Sized> Get for BibtexOutput<'r, W> {
    type Data = (BibtexEntry, Identifier);
}

pub struct TemplateOutput<'r, W: ?Sized, const CANONICAL: bool> {
    first: bool,
    strict: bool,
    template: Template,
    writer: &'r mut W,
    sep: &'r str,
}

impl<'r, W: io::Write + ?Sized, const CANONICAL: bool> TemplateOutput<'r, W, CANONICAL> {
    pub fn new(strict: bool, template: Template, writer: &'r mut W, sep: &'r str) -> Self {
        Self {
            first: true,
            strict,
            template,
            writer,
            sep,
        }
    }

    pub fn write_terminating_newline(&mut self) -> Result<(), io::Error> {
        if self.first {
            Ok(())
        } else {
            writeln!(self.writer)
        }
    }
}

impl<'r, W: io::Write + ?Sized, T: TemplateData, const CANONICAL: bool> Output<T>
    for TemplateOutput<'r, W, CANONICAL>
{
    fn write_item(&mut self, row: &T) -> Result<(), io::Error> {
        if self.strict && !self.template.has_keys_contained_in(row) {
            return Ok(());
        }
        if self.first {
            self.first = false;
        } else {
            write!(self.writer, "{}", self.sep)?;
        }
        self.template.render_io(&mut self.writer, row)
    }

    fn finish(mut self) -> Result<(), io::Error> {
        self.write_terminating_newline()
    }
}

impl<'a, Q, W> MapRow<Q> for TemplateOutput<'a, W, false>
where
    Q: col::Name + col::DataArbitrary + col::Canonical + col::Modified + col::DataEntry,
    W: io::Write + ?Sized,
{
    type Access<'r> = KeyedRecord<Record<&'r ArchivedEntryData, &'r str>, &'r str>;

    type Error = io::Error;

    fn map<'r>(&mut self, access: Self::Access<'r>) -> Result<(), Self::Error> {
        self.write_item(&access)
    }
}

impl<'r, W: io::Write + ?Sized> Get for TemplateOutput<'r, W, false> {
    type Data = KeyedRecord;
}

impl<'a, Q, W> MapRow<Q> for TemplateOutput<'a, W, true>
where
    Q: col::DataArbitrary + col::Canonical + col::Modified + col::DataEntry,
    W: io::Write + ?Sized,
{
    type Access<'r> = Record<&'r ArchivedEntryData, &'r str>;

    type Error = io::Error;

    fn map<'r>(&mut self, access: Self::Access<'r>) -> Result<(), Self::Error> {
        self.write_item(&access)
    }
}

pub fn retrieve_all<G, C>(
    mut writer: G,
    cfg: &Config,
    client: &C,
    record_db: &mut RecordDatabase,
    identifiers: Vec<Key>,
    ignore_null: bool,
) -> anyhow::Result<()>
where
    G: Get,
    C: Client,
{
    // then explicit arguments
    for id in identifiers {
        if let Some(item) =
            retrieve_single_entry(record_db, id, client, ignore_null, cfg, G::Data::filter_map)?
        {
            writer.write_item(&item)?;
        }
    }

    // then standard input
    let stdin = io::stdin().lock();
    if !stdin.is_terminal() {
        for line in stdin.lines() {
            let id = Key::from(line?);
            if let Some(item) =
                retrieve_single_entry(record_db, id, client, ignore_null, cfg, G::Data::filter_map)?
            {
                writer.write_item(&item)?;
            }
        }
    }

    writer.finish()?;
    Ok(())
}

pub fn retrieve_all_read_only<G: Get>(
    mut writer: G,
    cfg: &Config,
    record_db: &mut RecordDatabase,
    identifiers: Vec<Key>,
    ignore_null: bool,
) -> anyhow::Result<()> {
    // explicit arguments
    for id in identifiers {
        if let Some(item) =
            retrieve_single_entry_read_only(record_db, id, ignore_null, cfg, G::Data::filter_map)?
        {
            writer.write_item(&item)?;
        }
    }

    let stdin = io::stdin().lock();
    if !stdin.is_terminal() {
        for line in stdin.lines() {
            let id = Key::from(line?);
            if let Some(item) = retrieve_single_entry_read_only(
                record_db,
                id,
                ignore_null,
                cfg,
                G::Data::filter_map,
            )? {
                writer.write_item(&item)?;
            }
        }
    }
    writer.finish()?;
    Ok(())
}
