use std::{fmt, marker::PhantomData};

use crossterm::style::{ContentStyle, StyledContent, Stylize};
use ramify::TryRamify;

use super::{
    AsRecordKey, EntryDataOrReplacement, RecordContext, RecordKey, RecordRowData, RecordsLookup,
    State, Transaction,
};
use crate::entry::EntryData;

/// A specific version of a record row.
///
/// The lifetime is tied to the transaction in which this data is guaranteed to be valid.
pub struct Version<'tx, 'conn> {
    pub row: RecordContext,
    tx: &'tx Transaction<'conn>,
    row_id: i64,
}

pub struct FormatRev(i64);

impl fmt::Display for FormatRev {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:0>4x}", self.0)
    }
}

impl<'tx, 'conn> fmt::Display for Version<'tx, 'conn> {
    fn fmt(&self, buf: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hex = StyledContent::new(ContentStyle::default().yellow(), FormatRev(self.row_id));

        match &self.row.data {
            EntryDataOrReplacement::Entry(raw_entry_data) => {
                writeln!(buf, "  @{}{{{hex},", raw_entry_data.entry_type())?;
                for (key, val) in raw_entry_data.fields() {
                    writeln!(buf, "    {key} = {{{val}}},")?;
                }
                writeln!(buf, "  }}")?;

                Ok(())
            }
            EntryDataOrReplacement::Deleted(Some(remote_id)) => {
                write!(buf, "{hex} Replaced by '{remote_id}'")
            }
            EntryDataOrReplacement::Deleted(None) => write!(buf, "{hex} Deleted"),
        }
    }
}

impl<'tx, 'conn> Version<'tx, 'conn> {
    fn marker(&self, row_id: i64) -> char {
        match self.row.data {
            EntryDataOrReplacement::Entry(_) => {
                if self.row_id == row_id {
                    '◉'
                } else {
                    '○'
                }
            }
            EntryDataOrReplacement::Deleted(_) => {
                if self.row_id == row_id {
                    '⊗'
                } else {
                    '✕'
                }
            }
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
        let mut children = Vec::with_capacity(vtx.row.child_count());
        for row_id in vtx
            .row
            .children
            .as_chunks()
            .0
            .iter()
            .rev()
            .map(|chunk| i64::from_le_bytes(*chunk))
        {
            let child_row =
                match <RecordContext as RecordsLookup<RecordKey>>::lookup_unchecked(vtx.tx, row_id)
                {
                    Ok(row) => row,
                    Err(err) => return Err(ramify::Replacement { value: vtx, err }),
                };

            children.push(Version {
                row: child_row,
                tx: vtx.tx,
                row_id,
            });
        }
        Ok(children)
    }

    fn get_key(&self, vtx: &Version<'tx, 'conn>) -> impl Ord {
        &vtx.row.modified
    }

    fn marker(&self, vtx: &Version<'tx, 'conn>) -> char {
        vtx.marker(self.0)
    }

    fn annotation<B: fmt::Write>(&self, vtx: &Version<'tx, 'conn>, mut buf: B) -> fmt::Result {
        let disp = StyledContent::new(ContentStyle::default(), vtx);

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
        match vtx.row.parent {
            None => Ok(None.into_iter()),
            Some(parent_id) => {
                let parent_row = match <RecordContext as RecordsLookup<RecordKey>>::lookup_unchecked(
                    vtx.tx, parent_id,
                ) {
                    Ok(row) => row,
                    Err(err) => return Err(ramify::Replacement { value: vtx, err }),
                };

                Ok(Some(Version {
                    row: parent_row,
                    row_id: parent_id,
                    tx: vtx.tx,
                })
                .into_iter())
            }
        }
    }

    fn get_key(&self, vtx: &Version<'tx, 'conn>) -> impl Ord {
        &vtx.row.modified
    }

    fn marker(&self, vtx: &Version<'tx, 'conn>) -> char {
        vtx.marker(self.0)
    }

    fn annotation<B: fmt::Write>(&self, vtx: &Version<'tx, 'conn>, mut buf: B) -> fmt::Result {
        let disp = StyledContent::new(ContentStyle::default(), vtx);

        if vtx.row_id == self.0 {
            write!(buf, "{}", disp.bold())
        } else {
            write!(buf, "{disp}")
        }
    }
}

/// Changelog implementation
impl<'conn, I: AsRecordKey> State<'conn, I> {
    /// Determine the number of elements in the changelog to obtain an iteration bound.
    pub fn changelog_size(&self) -> rusqlite::Result<usize> {
        self.prepare("SELECT COUNT(*) FROM Records WHERE record_id = (SELECT record_id from Records WHERE key = ?1)")?
            .query_row((self.row_id(),), |row| row.get(0))
    }

    /// Get the parent ID of a row.
    fn parent_id(&self, row_id: i64) -> rusqlite::Result<Option<i64>> {
        self.prepare_cached("SELECT parent_key FROM Records WHERE key = ?1")?
            .query_row((row_id,), |row| row.get(0))
    }

    /// Get the ID of the root.
    fn root_id(&self) -> rusqlite::Result<i64> {
        let mut row_id = self.row_id();
        while let Some(parent) = self.parent_id(row_id)? {
            row_id = parent;
        }
        Ok(row_id)
    }

    pub fn current<'tx>(&'tx self) -> rusqlite::Result<Version<'tx, 'conn>> {
        let row_id = self.row_id();
        let row = <RecordContext as RecordsLookup<I>>::lookup_unchecked(&self.tx, row_id)?;
        Ok(Version {
            row,
            tx: &self.tx,
            row_id,
        })
    }

    /// Get the root version associated with this record row.
    pub fn root<'tx>(&'tx self) -> rusqlite::Result<Version<'tx, 'conn>> {
        let root_id = self.root_id()?;
        let row = <RecordContext as RecordsLookup<I>>::lookup_unchecked(&self.tx, root_id)?;
        Ok(Version {
            row,
            tx: &self.tx,
            row_id: root_id,
        })
    }

    pub fn ancestor_ramifier(&self) -> AncestorRamifier<'_> {
        AncestorRamifier(self.row_id(), PhantomData)
    }

    pub fn full_history_ramifier(&self) -> FullHistoryRamifier<'_> {
        FullHistoryRamifier(self.row_id(), PhantomData)
    }

    /// Traverse the ancestors from the current record to the root record, and
    /// apply the provided closure to every visited record.
    pub fn ancestors<F>(&self, mut f: F) -> rusqlite::Result<()>
    where
        F: FnMut(RecordRowData),
    {
        let mut row_id = self.row_id();
        let mut limit = self.changelog_size()?;

        while limit > 0 {
            limit -= 1;
            f(<RecordRowData as RecordsLookup<I>>::lookup_unchecked(
                &self.tx, row_id,
            )?);

            row_id = if let Some(parent) = self.parent_id(row_id)? {
                parent
            } else {
                return Ok(());
            }
        }
        panic!("Database changelog contains infinite loop");
    }
}
