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
        state::{IsEntry, RecordRow, State},
    },
    entry::{Entry, RawEntryData},
    error::DatabaseError,
    format::{Template, TemplateData},
    http::Client,
    record::{Record, RecordId, RemoteId},
};

pub trait Output {
    type Data;

    fn write_item(&mut self, item: Self::Data) -> Result<(), io::Error>;

    fn filter_map(
        record: Record<RawEntryData>,
        row: &State<'_, IsEntry>,
    ) -> Result<Option<Self::Data>, DatabaseError>;

    fn finish(&mut self) -> Result<(), io::Error>;
}

pub struct NoOutput;

impl Output for NoOutput {
    type Data = Infallible;

    fn filter_map(
        _: Record<RawEntryData>,
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
    type Data = (Entry<RawEntryData>, RemoteId);

    fn write_item(&mut self, (entry, _): Self::Data) -> Result<(), io::Error> {
        if self.first {
            self.first = false;
        } else {
            write!(self.writer, "\n\n")?;
        }

        entry.write_io(&mut self.writer)
    }

    fn filter_map(
        record: Record<RawEntryData>,
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
    type Data = Record<RawEntryData>;

    fn write_item(&mut self, row: Self::Data) -> Result<(), io::Error> {
        self.0.write_item(row)
    }

    fn filter_map(
        record: Record<RawEntryData>,
        _: &State<'_, IsEntry>,
    ) -> Result<Option<Self::Data>, DatabaseError> {
        Ok(Some(record))
    }

    fn finish(&mut self) -> Result<(), io::Error> {
        self.0.finish()
    }
}

pub struct TemplateRowOutput<'r, W: ?Sized>(TemplateOutputInner<'r, W>);

impl<'r, W: io::Write + ?Sized> TemplateRowOutput<'r, W> {
    pub fn new(strict: bool, template: Template, writer: &'r mut W, sep: &'r str) -> Self {
        Self(TemplateOutputInner::new(strict, template, writer, sep))
    }
}

impl<'r, W: io::Write + ?Sized> Output for TemplateRowOutput<'r, W> {
    type Data = RecordRow<RawEntryData>;

    fn write_item(&mut self, row: Self::Data) -> Result<(), io::Error> {
        self.0.write_item(row)
    }

    fn filter_map(
        record: Record<RawEntryData>,
        _: &State<'_, IsEntry>,
    ) -> Result<Option<Self::Data>, DatabaseError> {
        Ok(Some(record.row))
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
    identifiers: Vec<RecordId>,
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
            let id = RecordId::from(line?);
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
    identifiers: Vec<RecordId>,
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
            let id = RecordId::from(line?);
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
