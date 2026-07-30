use std::{
    convert::Infallible,
    io::IsTerminal,
    io::{self, BufRead},
};

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

pub trait Output {
    type Data;

    fn write_item(&mut self, item: Self::Data) -> Result<(), io::Error>;

    fn filter_map(
        record: KeyedRecord,
        row: &State<'_, IsEntry>,
    ) -> Result<Option<Self::Data>, DatabaseError>;

    fn finish(&mut self) -> Result<(), io::Error>;
}

pub struct NoOutput;

impl Output for NoOutput {
    type Data = Infallible;

    fn filter_map(
        _: KeyedRecord,
        _: &State<'_, IsEntry>,
    ) -> Result<Option<Self::Data>, DatabaseError> {
        Ok(None)
    }

    fn write_item(&mut self, data: Self::Data) -> Result<(), io::Error> {
        match data {}
    }

    fn finish(&mut self) -> Result<(), io::Error> {
        Ok(())
    }
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

impl<'r, W: io::Write + ?Sized> Output for BibtexOutput<'r, W> {
    type Data = (BibtexEntry, Identifier);

    fn write_item(&mut self, (entry, _): Self::Data) -> Result<(), io::Error> {
        if self.first {
            self.first = false;
        } else {
            write!(self.writer, "\n\n")?;
        }

        entry.write_io(&mut self.writer)
    }

    fn filter_map(
        record: KeyedRecord,
        row: &State<'_, IsEntry>,
    ) -> Result<Option<Self::Data>, DatabaseError> {
        Ok(retrieve::try_data_to_entry(record, row))
    }

    fn finish(&mut self) -> Result<(), io::Error> {
        if self.first {
            Ok(())
        } else {
            writeln!(self.writer)
        }
    }
}

struct TemplateOutputInner<'r, W: ?Sized> {
    first: bool,
    strict: bool,
    template: Template,
    writer: &'r mut W,
    sep: &'r str,
}

impl<'r, W: io::Write + ?Sized> TemplateOutputInner<'r, W> {
    fn new(strict: bool, template: Template, writer: &'r mut W, sep: &'r str) -> Self {
        Self {
            first: true,
            strict,
            template,
            writer,
            sep,
        }
    }

    fn write_item<T: TemplateData>(&mut self, row: T) -> Result<(), io::Error> {
        if self.strict && !self.template.has_keys_contained_in(&row) {
            return Ok(());
        }
        if self.first {
            self.first = false;
        } else {
            write!(self.writer, "{}", self.sep)?;
        }
        self.template.render_io(&mut self.writer, &row)
    }

    fn finish(&mut self) -> Result<(), io::Error> {
        if self.first {
            Ok(())
        } else {
            writeln!(self.writer)
        }
    }
}

pub struct TemplateOutput<'r, W: ?Sized>(TemplateOutputInner<'r, W>);

impl<'r, W: io::Write + ?Sized> TemplateOutput<'r, W> {
    pub fn new(strict: bool, template: Template, writer: &'r mut W, sep: &'r str) -> Self {
        Self(TemplateOutputInner::new(strict, template, writer, sep))
    }
}

impl<'r, W: io::Write + ?Sized> Output for TemplateOutput<'r, W> {
    type Data = KeyedRecord;

    fn write_item(&mut self, row: Self::Data) -> Result<(), io::Error> {
        self.0.write_item(row)
    }

    fn filter_map(
        record: KeyedRecord,
        _: &State<'_, IsEntry>,
    ) -> Result<Option<Self::Data>, DatabaseError> {
        Ok(Some(record))
    }

    fn finish(&mut self) -> Result<(), io::Error> {
        self.0.finish()
    }
}

impl<'a, Q, W> MapRow<Q> for TemplateOutput<'a, W>
where
    Q: col::Name + col::DataArbitrary + col::Canonical + col::Modified + col::DataEntry,
    W: io::Write + ?Sized,
{
    type Access<'r> = KeyedRecord;

    type Error = io::Error;

    fn map<'r>(&mut self, access: Self::Access<'r>) -> Result<(), Self::Error> {
        self.write_item(access)
    }
}

pub struct TemplateRowOutput<'r, W: ?Sized>(TemplateOutputInner<'r, W>);

impl<'a, Q, W> MapRow<Q> for TemplateRowOutput<'a, W>
where
    Q: col::DataArbitrary + col::Canonical + col::Modified + col::DataEntry,
    W: io::Write + ?Sized,
{
    type Access<'r> = Record;

    type Error = io::Error;

    fn map<'r>(&mut self, access: Self::Access<'r>) -> Result<(), Self::Error> {
        self.write_item(access)
    }
}

impl<'r, W: io::Write + ?Sized> TemplateRowOutput<'r, W> {
    pub fn new(strict: bool, template: Template, writer: &'r mut W, sep: &'r str) -> Self {
        Self(TemplateOutputInner::new(strict, template, writer, sep))
    }
}

impl<'r, W: io::Write + ?Sized> Output for TemplateRowOutput<'r, W> {
    type Data = Record;

    fn write_item(&mut self, row: Self::Data) -> Result<(), io::Error> {
        self.0.write_item(row)
    }

    fn filter_map(
        record: KeyedRecord,
        _: &State<'_, IsEntry>,
    ) -> Result<Option<Self::Data>, DatabaseError> {
        Ok(Some(record.record))
    }

    fn finish(&mut self) -> Result<(), io::Error> {
        self.0.finish()
    }
}

pub fn retrieve_all<W, C>(
    mut writer: W,
    cfg: &Config,
    client: &C,
    record_db: &mut RecordDatabase,
    identifiers: Vec<Key>,
    ignore_null: bool,
) -> anyhow::Result<()>
where
    W: Output,
    C: Client,
{
    // then explicit arguments
    for id in identifiers {
        if let Some(item) =
            retrieve_single_entry(record_db, id, client, ignore_null, cfg, W::filter_map)?
        {
            writer.write_item(item)?;
        }
    }

    // then standard input
    let stdin = io::stdin().lock();
    if !stdin.is_terminal() {
        for line in stdin.lines() {
            let id = Key::from(line?);
            if let Some(item) =
                retrieve_single_entry(record_db, id, client, ignore_null, cfg, W::filter_map)?
            {
                writer.write_item(item)?;
            }
        }
    }

    writer.finish()?;
    Ok(())
}

pub fn retrieve_all_read_only<W>(
    mut writer: W,
    cfg: &Config,
    record_db: &mut RecordDatabase,
    identifiers: Vec<Key>,
    ignore_null: bool,
) -> anyhow::Result<()>
where
    W: Output,
{
    // explicit arguments
    for id in identifiers {
        if let Some(item) =
            retrieve_single_entry_read_only(record_db, id, ignore_null, cfg, W::filter_map)?
        {
            writer.write_item(item)?;
        }
    }

    let stdin = io::stdin().lock();
    if !stdin.is_terminal() {
        for line in stdin.lines() {
            let id = Key::from(line?);
            if let Some(item) =
                retrieve_single_entry_read_only(record_db, id, ignore_null, cfg, W::filter_map)?
            {
                writer.write_item(item)?;
            }
        }
    }
    writer.finish()?;
    Ok(())
}
