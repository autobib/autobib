//! # Database state representation
//! This module implements the [typestate pattern](https://cliffle.com/blog/rust-typestate/) to
//! represent the internal database state as corresponds to a given [`RecordId`].
//!
//! The [`State`] struct is a representation of the database state corresponding to a [`RecordId`].
//! Internally, the [`State`] struct is a wrapper around a [`Transaction`], which ensures
//! that the underlying database state will not change during the running of this program.
//!
//! A [`RecordId`] is represented by the database in exactly one of the following ways, which is
//! represented by the corresponding implementation of [`State`].
//!
//! 1. The [`RecordId`] corresponds to a row in the `Records` table, and is not present in the
//!    `NullRecords` table. This is [`State<RecordRow`>].
//! 2. The [`RecordId`] corresponds to a row in the `NullRecords` table, and is not present in the
//!    `Records` table. This is [`State<NullRecordRow>`].
//! 3. The [`RecordId`] is not present in either the `Records` table or the `NullRecords` table.
//!    This is [`State<Missing>`].
//!
//! Any implementation of [`State`] has access to the following operations:
//!
//! - [`commit`](State::commit) the state, which writes the relevant changes to the database.
//! - [`reset`](State::reset) the state, which re-associates the internal transaction with a new
//!   state. Since the underlying transaction is preserved, it must be committed even for the
//!   changes prior to the [`reset`](State::reset) takes place. As a result,
//!   [`reset`](State::reset) should be used with care to avoid lost data!
//!
//! If the state has an associated row, this is represented by the [`InDatabase`] trait, which
//! gives access to the internal [`State::row_id`] method, which returns the internal [`RowId`] of
//! the corresponding row.
//!
//! The different states can be converted to each other.
//!
//! | From                     | To                       | Method              |
//! |--------------------------|--------------------------|---------------------|
//! | [`State<RecordRow>`]     | [`State<Missing>`]       | [`State::delete`]   |
//! | [`State<NullRecordRow>`] | [`State<Missing>`]       | [`State::delete`]   |
//! | [`State<Missing>`]       | [`State<RecordRow>`]     | [`State::insert`]   |
//! | [`State<Missing>`]       | [`State<NullRecordRow>`] | [`State::set_null`] |
//!
//! Each of the particular implementation of [`State`] also supports a number of additional methods
//! which are relevant database operations in the provided state.
mod missing;
mod null;
mod record;

use rusqlite::{CachedStatement, Error, Statement};

pub use self::{missing::*, null::*, record::*};
use super::{get_null_row_id, get_row_id, RowId, Transaction};
use crate::{
    error::RecordError, logger::debug, Alias, AliasOrRemoteId, MappedKey, RecordId, RemoteId,
};

/// A representation of the current database state corresponding to a [`RecordId`].
#[derive(Debug)]
pub struct State<'conn, I: DatabaseId> {
    tx: Transaction<'conn>,
    id: I,
}

/// A trait which represents a database id, which can either be present or missing.
pub trait DatabaseId: private::Sealed {}

/// A trait which represents a [`DatabaseId`] which is present in the database.
pub trait InDatabase: DatabaseId + private::Sealed {
    /// The type of data that is associated with the row, which can be read from a
    /// [`rusqlite::Row`].
    type Data: for<'a, 'conn> TryFrom<&'a rusqlite::Row<'conn>, Error = rusqlite::Error>;

    /// The statement to get the data associated with the row.
    const GET_STMT: &str;

    /// The statement to delete the corresponding row.
    const DELETE_STMT: &str;

    /// Get the [`RowId`] corresponding to the row.
    fn row_id(&self) -> RowId;

    /// Construct from a [`RowId`] corresponding to a given row.
    fn from_row_id(id: RowId) -> Self;
}

mod private {
    use super::{Missing, NullRecordRow, RecordRow};

    pub trait Sealed {}

    impl Sealed for RecordRow {}
    impl Sealed for NullRecordRow {}
    impl Sealed for Missing {}
}

impl<'conn, I: DatabaseId> State<'conn, I> {
    /// Reset the row, clearing any internal data but preserving the transaction.
    pub fn reset(self, remote_id: &RemoteId) -> Result<RemoteIdState<'conn>, rusqlite::Error> {
        RemoteIdState::determine(self.tx, remote_id)
    }

    /// Commit the [`State`], writing the relevant changes to the database.
    pub fn commit(self) -> Result<(), Error> {
        self.tx.commit()
    }

    /// Initialize a new state given a transaction and a [`DatabaseId`] implementation.
    fn init(tx: Transaction<'conn>, id: I) -> Self {
        Self { tx, id }
    }

    /// # Safety
    /// The caller must ensure that a statement which included an insert was previously
    /// executed on the same transaction.
    unsafe fn into_last_insert<J: InDatabase>(self) -> State<'conn, J> {
        let Self { tx, .. } = self;
        let id = tx.last_insert_rowid();
        State::<'conn, J>::init(tx, <J as InDatabase>::from_row_id(id))
    }

    /// Prepare the SQL statement for execution.
    fn prepare(&self, sql: &'static str) -> Result<Statement, Error> {
        self.tx.prepare(sql)
    }

    /// Prepare the SQL statement for execution, caching the statement internally for more
    /// efficient subsequent calls.
    ///
    /// Note that the caching only exists in memory: internally, rusqlite uses
    /// hashlink's
    /// [`LruCache`](https://docs.rs/hashlink/latest/hashlink/lru_cache/struct.LruCache.html) to
    /// store statements. As a result, statement caching is only valuable for statements which
    /// are called many times in a single program run.
    ///
    /// Unfortunately, rusqlite does not support compile-time pre-caching of SQLite statements.
    fn prepare_cached(&self, sql: &'static str) -> Result<CachedStatement, Error> {
        self.tx.prepare_cached(sql)
    }
}

impl<'conn, I: InDatabase> State<'conn, I> {
    /// Get the internal row-id, for use in SQL statements.
    fn row_id(&self) -> RowId {
        self.id.row_id()
    }

    /// Delete the row.
    pub fn delete(self) -> Result<State<'conn, Missing>, rusqlite::Error> {
        debug!("Deleting row '{}'", self.row_id());
        self.prepare(<I as InDatabase>::DELETE_STMT)?
            .execute((self.row_id(),))?;
        let Self { tx, .. } = self;
        Ok(State::init(tx, Missing {}))
    }

    /// Get the data associated with the row.
    pub fn get_data(&self) -> Result<<I as InDatabase>::Data, rusqlite::Error> {
        debug!("Retrieving data associated with row '{}'", self.row_id());
        self.prepare_cached(<I as InDatabase>::GET_STMT)?
            .query_row([self.row_id()], |row| row.try_into())
    }
}

/// A representation of the database state beginning with an arbitrary [`RecordId`].
#[derive(Debug)]
pub enum RecordIdState<'conn> {
    /// The `Records` row exists.
    Existent(String, State<'conn, RecordRow>),
    /// The `Records` row does not exist and the `NullRecords` row exists.
    NullRemoteId(MappedKey<RemoteId>, State<'conn, NullRecordRow>),
    /// The `Records` and `NullRecords` rows do not exist.
    UnknownRemoteId(MappedKey<RemoteId>, State<'conn, Missing>),
    /// The alias is undefined.
    UndefinedAlias(Alias),
    /// The remote id is invalid.
    InvalidRemoteId(RecordError),
}

impl<'conn> RecordIdState<'conn> {
    /// Determine the current state of the database, as corresponds to the provided record
    /// identifier.
    #[inline]
    pub fn determine(tx: Transaction<'conn>, record_id: RecordId) -> Result<Self, rusqlite::Error> {
        Ok(match get_row_id(&tx, &record_id)? {
            Some(row_id) => {
                debug!("Beginning new transaction for row '{row_id}' in the `Records` table.");
                RecordIdState::Existent(
                    record_id.into(),
                    State::init(tx, RecordRow::from_row_id(row_id)),
                )
            }
            None => match record_id.resolve() {
                Ok(AliasOrRemoteId::RemoteId(mapped_remote_id)) => {
                    // if it was normalized during name resolution, we need to check the normalized
                    // value as well
                    if mapped_remote_id.is_mapped() {
                        if let Some(row_id) = get_row_id(&tx, &mapped_remote_id)? {
                            debug!("Beginning new transaction for row '{row_id}' in the `Records` table.");
                            return Ok(RecordIdState::Existent(
                                mapped_remote_id.into(),
                                State::init(tx, RecordRow::from_row_id(row_id)),
                            ));
                        }
                    }

                    match get_null_row_id(&tx, &mapped_remote_id.key)? {
                        Some(row_id) => {
                            debug!("Beginning new transaction for row '{row_id}' in the `NullRecords` table.");
                            RecordIdState::NullRemoteId(
                                mapped_remote_id,
                                State::init(tx, NullRecordRow::from_row_id(row_id)),
                            )
                        }
                        None => {
                            debug!("Beginning new transaction for unknown remote id.");
                            RecordIdState::UnknownRemoteId(
                                mapped_remote_id,
                                State::init(tx, Missing {}),
                            )
                        }
                    }
                }
                Ok(AliasOrRemoteId::Alias(alias)) => {
                    tx.commit()?;
                    RecordIdState::UndefinedAlias(alias)
                }
                Err(record_error) => {
                    tx.commit()?;
                    RecordIdState::InvalidRemoteId(record_error)
                }
            },
        })
    }
}

/// A representation of the database state beginning with an arbitrary [`RemoteId`].
#[derive(Debug)]
pub enum RemoteIdState<'conn> {
    /// The `Records` row exists.
    Existent(State<'conn, RecordRow>),
    /// The `Records` row does not exist and the `NullRecords` row exists.
    Null(State<'conn, NullRecordRow>),
    /// The `Records` and `NullRecords` rows do not exist.
    Unknown(State<'conn, Missing>),
}

impl<'conn> RemoteIdState<'conn> {
    /// Determine the current state of the database, as corresponds to the provided remote record
    /// identifier.
    #[inline]
    pub fn determine(
        tx: Transaction<'conn>,
        remote_id: &RemoteId,
    ) -> Result<Self, rusqlite::Error> {
        Ok(match get_row_id(&tx, remote_id)? {
            Some(row_id) => {
                debug!("Beginning new transaction for row '{row_id}' in the `Records` table.");
                Self::Existent(State::init(tx, RecordRow::from_row_id(row_id)))
            }
            None => {
                match get_null_row_id(&tx, remote_id)? {
                    Some(row_id) => {
                        debug!("Beginning new transaction for row '{row_id}' in the `NullRecords` table.");
                        Self::Null(State::init(tx, NullRecordRow::from_row_id(row_id)))
                    }
                    None => {
                        debug!("Beginning new transaction for unknown remote id.");
                        Self::Unknown(State::init(tx, Missing {}))
                    }
                }
            }
        })
    }

    /// Extract the [`RecordRow`] if possible, and otherwise return [`None`].
    pub fn exists(self) -> Option<State<'conn, RecordRow>> {
        match self {
            RemoteIdState::Existent(record_row) => Some(record_row),
            RemoteIdState::Null(null_row) => {
                drop(null_row);
                None
            }
            RemoteIdState::Unknown(missing) => {
                drop(missing);
                None
            }
        }
    }
}
