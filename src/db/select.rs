//! # Abstractions over `SELECT` statements
//!
//! This module provides abstractions over `SELECT` statements. `SELECT` statements, via `rusqlite`
//! are untyped. This module provides a typed interface for `SELECT` statements, split into two
//! components:
//!
//! - Data types which can be read from rows returned by a `SELECT` statement implement
//!   [`AccessRow<'row, Q>`]. The lifetime `'row` is managed by SQLite and is very short lived,
//!   but sometimes this is manageable (e.g., for immediately printing data read directly
//!   from the database).
//!   The type parameter `Q` is denotes a `SELECT` statement from which the type can be read.
//! - The typed interface for a `SELECT` statement is provided by the [`SelectStatement`] trait.
//!   This includes the arguments that the statement requires to execute.
//!
//! A data type can only read data from a [`SelectStatement`] if the statement implements the
//! corresponding trait bounds from the [`col`] module. Loosely speaking, each trait corresponds to
//! a single trait in the database, so data types can always be read from statements which provide
//! additional data beyond what the column requires to load. This is especially convenient for
//! specifying trait bounds for compound data, since the compound data type only requires that each
//! composite component can read from the data.
//!
//! For a convenient interface for execute `SELECT` statements, see the [`Select`] trait.
//! This trait is implemented by [`RecordDatabase`] and [`Snapshot`].
mod access;
pub mod col;
pub mod stmt;

use core::ops::Deref;
use std::{error, fmt, marker::PhantomData};

use nucleo_picker::{Injector, Render};
use rusqlite::{Connection, OptionalExtension, Row};

use crate::{
    db::{Record, RecordDatabase, Snapshot, state::RevisionId},
    logger::debug,
};

mod unchecked {
    use rusqlite::{Connection, Row};

    use super::{AccessRow, SelectStatement};
    use crate::db::state::RevisionId;
    use core::ops::Deref;

    pub trait AccessRowUnchecked<'row>: Sized {
        fn access_row_unchecked(row: &'row Row<'_>) -> Self;
    }

    pub trait SelectOneUnchecked: SelectStatement<Args<'static> = RevisionId> {
        fn select_one_unchecked<R, Conn>(tx: &Conn, id: i64) -> Result<R, rusqlite::Error>
        where
            R: for<'r> AccessRow<'r, Self>,
            Conn: Deref<Target = Connection>,
        {
            Self::select_one_unchecked_cast(tx, id)
        }

        fn select_one_unchecked_cast<R, Conn>(tx: &Conn, id: i64) -> Result<R, rusqlite::Error>
        where
            R: for<'r> AccessRowUnchecked<'r>,
            Conn: Deref<Target = Connection>,
        {
            tx.prepare_cached(Self::STATEMENT)?
                .query_row((id,), |row| Ok(R::access_row_unchecked(row)))
        }
    }
}

pub(in crate::db) use unchecked::{AccessRowUnchecked, SelectOneUnchecked};

/// A typed `SELECT` statement.
pub trait SelectStatement {
    /// The arguments required by the `SELECT` statement.
    type Args<'a>;

    /// The raw statement.
    const STATEMENT: &str;

    /// A mapping which converts typed arguments to parameters of the SQL statement.
    fn args_to_params(args: Self::Args<'_>) -> impl rusqlite::Params;

    /// Apply a [`MapRow`] to each row returned by the provided statement.
    fn select_map<R, Conn>(
        tx: &Conn,
        mut f: R,
        args: Self::Args<'_>,
    ) -> Result<(), SelectErr<R::Error>>
    where
        R: MapRow<Self>,
        Conn: Deref<Target = rusqlite::Connection>,
    {
        debug!("Executing SQL statement: {}", Self::STATEMENT);
        let mut retriever = tx.prepare(Self::STATEMENT)?;
        let mut rows = retriever.query(Self::args_to_params(args))?;
        while let Some(row) = rows.next()? {
            f.map(R::Access::access_row(row))
                .map_err(SelectErr::MapFailed)?;
        }
        Ok(())
    }
}

impl<T: SelectOneUnchecked> SelectOne for T {}

/// `SELECT` statements which return at most one row from the Records table.
///
/// These statements are essentially of the form `SELECT ... FROM Records WHERE rev = ?1`.
pub trait SelectOne: SelectOneUnchecked {
    fn select_one<R, Conn>(tx: &Conn, rev: RevisionId) -> Result<Option<R>, rusqlite::Error>
    where
        R: for<'r> AccessRow<'r, Self>,
        Conn: Deref<Target = rusqlite::Connection>,
    {
        Self::select_one_unchecked(tx, rev.0).optional()
    }
}

/// An error which may occur while applying a [`MapRow`] or a closure to the returned rows of a
/// `SELECT` statement.
#[derive(Debug)]
pub enum SelectErr<E> {
    /// There was an underlying database error.
    DatabaseError(rusqlite::Error),
    /// The closure itself returned an error.
    MapFailed(E),
}

impl<E> From<rusqlite::Error> for SelectErr<E> {
    fn from(err: rusqlite::Error) -> Self {
        Self::DatabaseError(err)
    }
}

impl From<SelectErr<std::convert::Infallible>> for rusqlite::Error {
    fn from(err: SelectErr<std::convert::Infallible>) -> Self {
        let SelectErr::DatabaseError(inner) = err;
        inner
    }
}

impl<E: fmt::Display> fmt::Display for SelectErr<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MapFailed(error) => error.fmt(f),
            Self::DatabaseError(error) => error.fmt(f),
        }
    }
}

impl<E: error::Error> error::Error for SelectErr<E> {}

/// Types which can perform some action on each row returned by a `SELECT` statement.
///
/// This is essentially a `FnMut(Row) -> Result<(), Error>`, but implemented as a trait since the
/// lifetime bounds cannot be expressed at the moment.
pub trait MapRow<Q: ?Sized> {
    /// The data type which is loaded from the row.
    type Access<'r>: AccessRow<'r, Q>;

    /// An error which may occur while processing the row.
    type Error;

    /// Process the row.
    fn map<'r>(&mut self, access: Self::Access<'r>) -> Result<(), Self::Error>;
}

impl<Q: ?Sized, T: MapRow<Q>> MapRow<Q> for &mut T {
    type Access<'r> = T::Access<'r>;

    type Error = T::Error;

    fn map<'r>(&mut self, access: Self::Access<'r>) -> Result<(), Self::Error> {
        (*self).map(access)
    }
}

/// A convenient wrapper implementing [`MapRow`] for closures which do not capture any lifetimes
/// from the returned row.
pub struct FnMutMap<F, R, E> {
    pub map: F,
    _marker: PhantomData<(R, E)>,
}

impl<F, R, E> FnMutMap<F, R, E>
where
    F: FnMut(R) -> Result<(), E>,
{
    pub fn new(map: F) -> Self {
        Self {
            map,
            _marker: PhantomData,
        }
    }
}

impl<Q, F, R, E> MapRow<Q> for FnMutMap<F, R, E>
where
    F: FnMut(R) -> Result<(), E>,
    R: for<'r> AccessRow<'r, Q>,
{
    type Access<'r> = R;

    type Error = E;

    fn map<'r>(&mut self, access: Self::Access<'r>) -> Result<(), Self::Error> {
        (self.map)(access)
    }
}

/// A trait for types which can access the data in a row in the Records table of the records database.
///
/// The type parameter `Q` denotes the type of statement that this struct can read from.
pub trait AccessRow<'row, Q: ?Sized>: AccessRowUnchecked<'row> {
    fn access_row(row: &'row Row<'_>) -> Self {
        Self::access_row_unchecked(row)
    }
}

pub trait Select {
    fn conn(&self) -> &Connection;

    /// Execute a [`SelectStatement`] against this connection with the provided callback and
    /// statement parameters.
    ///
    /// This is a convenience wrapper around [`select_ref`](Self::select_ref) which accepts a
    /// closure, as long as the closure does not capture any lifetimes from the queried value `R`.
    /// For types `R` which may borrow from `'row`, implement [`MapRow`] and use
    /// [`select_ref`](Self::select_ref).
    fn select<Stmt, R, E, F>(&self, f: F, params: Stmt::Args<'_>) -> Result<(), SelectErr<E>>
    where
        Stmt: SelectStatement,
        F: FnMut(R) -> Result<(), E>,
        R: for<'r> AccessRow<'r, Stmt>,
    {
        self.select_ref(FnMutMap::new(f), params)
    }

    /// Execute a [`SelectStatement`] against this connection with the provided [map](MapRow)
    /// and statement parameters.
    fn select_ref<Stmt, M>(&self, map: M, params: Stmt::Args<'_>) -> Result<(), SelectErr<M::Error>>
    where
        Stmt: SelectStatement,
        M: MapRow<Stmt>,
    {
        Stmt::select_map(&self.conn(), map, params)
    }

    /// Send the active rows in the `Records` table to a [`Picker`](`nucleo_picker::Picker`)
    /// via its [`Injector`].
    ///
    /// The provided `filter_map` closure plays a similar role to [`Iterator::filter_map`]
    /// by transforming a [`Record`] into the picker item type, with the option to exclude
    /// the item from being sent to the matcher entirely by returning [`None`].
    ///
    /// This is a convenience wrapper around [`select`](Self::select) with statement
    /// [`SelectActiveRecords`](stmt::SelectActiveRecords).
    fn inject_active_records<T, F, R>(
        &self,
        injector: Injector<T, R>,
        mut filter_map: F,
    ) -> Result<(), rusqlite::Error>
    where
        F: FnMut(Record) -> Option<T>,
        R: Render<T>,
    {
        self.select::<stmt::SelectActiveRecords, _, _, _>(
            |res| {
                if let Some(data) = filter_map(res) {
                    injector.push(data);
                }
                Ok(())
            },
            (),
        )?;
        Ok(())
    }
}

impl Select for RecordDatabase {
    fn conn(&self) -> &Connection {
        &self.conn
    }
}

impl Select for Snapshot<'_> {
    fn conn(&self) -> &Connection {
        self.tx.inner_connection()
    }
}
