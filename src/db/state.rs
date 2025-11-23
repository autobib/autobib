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
//! The different states can be converted to each other.
//!
//! | From                      | To                       | Method              |
//! |---------------------------|--------------------------|---------------------|
//! | [`State<RecordRow>`] | [`State<Missing>`]       | [`State::delete`]   |
//! | [`State<NullRecordRow>`]  | [`State<Missing>`]       | [`State::delete`]   |
//! | [`State<Missing>`]        | [`State<RecordRow>`]     | [`State::insert`]   |
//! | [`State<Missing>`]        | [`State<NullRecordRow>`] | [`State::set_null`] |
//!
//! Each of the particular implementation of [`State`] also supports a number of additional methods
//! which are relevant database operations in the provided state.
mod missing;
mod null;
mod record;
pub mod revisions;

use rusqlite::{CachedStatement, Error, Statement};

pub use self::{missing::*, null::*, record::*};
use super::{RowId, Transaction, get_null_row_id, get_row_id};
use crate::{
    Alias, AliasOrRemoteId, MappedKey, RecordId, RemoteId,
    config::AliasTransform,
    error::RecordError,
    logger::{debug, error, reraise},
};

/// A representation of the current database state corresponding to a [`RecordId`].
#[derive(Debug)]
pub struct State<'conn, I> {
    tx: Transaction<'conn>,
    id: I,
}

impl<'conn, I> State<'conn, I> {
    /// Reset the row, clearing any internal data but preserving the transaction.
    pub fn reset(self, remote_id: &RemoteId) -> Result<RemoteIdState<'conn>, rusqlite::Error> {
        RemoteIdState::determine(self.tx, remote_id)
    }

    /// Commit the [`State`], writing the relevant changes to the database.
    pub fn commit(self) -> Result<(), Error> {
        self.tx.commit()
    }

    /// Initialize a new state from a transaction.
    fn init(tx: Transaction<'conn>, id: I) -> Self {
        Self { tx, id }
    }

    /// Prepare the SQL statement for execution.
    fn prepare(&self, sql: &'static str) -> Result<Statement<'_>, Error> {
        self.tx.prepare(sql)
    }

    /// Prepare the SQL statement for execution, caching the statement internally for more
    /// efficient subsequent calls.
    ///
    /// The caching only exists in memory, so caching is only worthwhile for statements
    /// which are called many times in a single program run.
    fn prepare_cached(&self, sql: &'static str) -> Result<CachedStatement<'_>, Error> {
        self.tx.prepare_cached(sql)
    }
}

/// A record does not exist in the `Records` or `NullRecords` table.
#[derive(Debug)]
pub enum Unknown<'conn> {
    /// The record was originally an alias, and it was mapped to the given remote identifier.
    MappedAlias(Alias, RemoteId, State<'conn, Missing>),
    /// The record was a remote identifier, possibly with a transformation applied.
    RemoteId(MappedKey, State<'conn, Missing>),
}

impl Unknown<'_> {
    /// Commit the [`Missing`] transaction, and convert the underlying data into a [`MappedKey`].
    pub fn combine_and_commit(self) -> Result<MappedKey, rusqlite::Error> {
        match self {
            Unknown::MappedAlias(alias, remote_id, state) => {
                state.commit()?;
                Ok(MappedKey::mapped(remote_id, alias.into()))
            }
            Unknown::RemoteId(mapped_key, state) => {
                state.commit()?;
                Ok(mapped_key)
            }
        }
    }
}

/// A representation of the database state beginning with an arbitrary [`RecordId`].
#[derive(Debug)]
pub enum RecordIdState<'conn> {
    /// The `Records` row exists.
    Entry(String, EntryRowData, State<'conn, EntryRecordKey>),
    /// The `Records` row was deleted.
    Deleted(String, DeletedRowData, State<'conn, DeletedRecordKey>),
    /// The `Records` row does not exist and the `NullRecords` row exists.
    NullRemoteId(MappedKey, State<'conn, NullRecordRow>),
    /// The `Records` and `NullRecords` rows do not exist.
    Unknown(Unknown<'conn>),
    /// The alias is undefined.
    UndefinedAlias(Alias),
    /// The remote id is invalid.
    InvalidRemoteId(RecordError),
}

impl<'conn> RecordIdState<'conn> {
    /// Create a new `Existent` variant from the provided [`Transaction`] and [`RowId`], using the
    /// provided key as the original key for the request.
    fn existent(
        tx: Transaction<'conn>,
        row_id: RowId,
        key: impl Into<String>,
    ) -> Result<Self, rusqlite::Error> {
        Self::existent_with_callback(tx, row_id, |_, s| Ok(s.into()), |_, s| Ok(s.into()), key)
    }

    /// Create a new `Existent` variant from the provided [`Transaction`] and [`RowId`], using the
    /// provided callback to perform an action on the resulting row if it exists or if it was
    /// deleted.
    fn existent_with_callback<K>(
        tx: Transaction<'conn>,
        row_id: RowId,
        produce_key_entry: impl FnOnce(
            &State<'conn, EntryRecordKey>,
            K,
        ) -> Result<String, rusqlite::Error>,
        produce_key_deleted: impl FnOnce(
            &State<'conn, DeletedRecordKey>,
            K,
        ) -> Result<String, rusqlite::Error>,
        key: K,
    ) -> Result<Self, rusqlite::Error> {
        debug!("Beginning new transaction for row '{row_id}' in the `Records` table.");
        match State::init(tx, RecordKey(row_id)).determine()? {
            EntryOrDeletedRow::Entry(entry_row_data, state) => {
                let key = produce_key_entry(&state, key)?;
                Ok(Self::Entry(key, entry_row_data, state))
            }
            EntryOrDeletedRow::Deleted(deleted_row_data, state) => {
                let key = produce_key_deleted(&state, key)?;
                Ok(Self::Deleted(key, deleted_row_data, state))
            }
        }
    }

    /// Match on the remote id determined from the context `id_from_context`. If the corresponding
    /// `NullRecords` row exists, return a `NullRemoteId` constructed from the [`MappedKey`]
    /// value returned by `produce_null`. Otherwise, produce a different variant using the context
    /// and the [`State<Missing>`] database state.
    fn null_or_missing<C>(
        tx: Transaction<'conn>,
        context: C,
        id_from_context: impl for<'a> FnOnce(&'a C) -> &'a RemoteId,
        produce_null: impl FnOnce(C) -> MappedKey,
        produce_missing: impl FnOnce(C, State<'conn, Missing>) -> Self,
    ) -> Result<Self, rusqlite::Error> {
        match get_null_row_id(&tx, id_from_context(&context))? {
            Some(row_id) => {
                debug!("Beginning new transaction for row '{row_id}' in the `NullRecords` table.");
                Ok(Self::NullRemoteId(
                    produce_null(context),
                    State::init(tx, NullRecordRow(row_id)),
                ))
            }
            None => {
                debug!("Beginning new transaction for unknown remote id.");
                Ok(produce_missing(context, State::init(tx, Missing {})))
            }
        }
    }

    /// Determine the current state of the database, as corresponds to the provided record
    /// identifier.
    pub fn determine<A: AliasTransform>(
        tx: Transaction<'conn>,
        record_id: RecordId,
        alias_transform: &A,
    ) -> Result<Self, rusqlite::Error> {
        // fast path when the identifier is already a citation key in the table
        if let Some(row_id) = get_row_id(&tx, &record_id)? {
            return Self::existent(tx, row_id, record_id);
        };

        match record_id.resolve(alias_transform) {
            Ok(AliasOrRemoteId::RemoteId(mapped_remote_id)) => {
                // check the normalized value, if normalized
                if mapped_remote_id.is_mapped()
                    && let Some(row_id) = get_row_id(&tx, &mapped_remote_id)?
                {
                    return Self::existent(tx, row_id, mapped_remote_id);
                }

                Self::null_or_missing(
                    tx,
                    mapped_remote_id,
                    |ctx| &ctx.mapped,
                    |ctx| ctx,
                    |ctx, m| Self::Unknown(Unknown::RemoteId(ctx, m)),
                )
            }
            Ok(AliasOrRemoteId::Alias(alias, maybe_mapped)) => {
                // check the mapped value, if mapped
                match maybe_mapped {
                    Some(remote_id) => {
                        if let Some(row_id) = get_row_id(&tx, &remote_id)? {
                            return Self::existent_with_callback(
                                tx,
                                row_id,
                                |row, alias| {
                                    if alias_transform.create() {
                                        row.add_alias(&alias)?;
                                    }
                                    Ok(alias.into())
                                },
                                |_, alias| Ok(alias.into()),
                                alias,
                            );
                        }

                        Self::null_or_missing(
                            tx,
                            (remote_id, alias),
                            |ctx| &ctx.0,
                            |ctx| MappedKey::mapped(ctx.0, ctx.1.into()),
                            |ctx, m| Self::Unknown(Unknown::MappedAlias(ctx.1, ctx.0, m)),
                        )
                    }
                    None => {
                        tx.commit()?;
                        Ok(Self::UndefinedAlias(alias))
                    }
                }
            }
            Err(record_error) => {
                tx.commit()?;
                Ok(Self::InvalidRemoteId(record_error))
            }
        }
    }

    /// Combine the entry and deleted data into a unified `RecordRow` state, and commit any other
    /// states, reporting an error to standard error.
    pub fn flatten(
        self,
    ) -> Result<Option<(String, RecordRowData, State<'conn, RecordKey>)>, rusqlite::Error> {
        Ok(match self {
            Self::Entry(s, data, state) => Some((s, data.into(), state.forget())),
            Self::Deleted(s, data, state) => Some((s, data.into(), state.forget())),
            Self::NullRemoteId(mapped_key, state) => {
                state.commit()?;
                error!("Null remote id: {mapped_key}");
                None
            }
            Self::Unknown(unknown) => {
                let maybe_normalized = unknown.combine_and_commit()?;
                error!("Record not in database: {maybe_normalized}");
                None
            }
            Self::UndefinedAlias(alias) => {
                error!("Undefined alias: '{alias}'");
                None
            }
            Self::InvalidRemoteId(record_error) => {
                reraise(&record_error);
                None
            }
        })
    }
}

/// A representation of the database state beginning with an arbitrary [`RemoteId`].
#[derive(Debug)]
pub enum RemoteIdState<'conn> {
    /// The `Records` row exists.
    Entry(EntryRowData, State<'conn, EntryRecordKey>),
    /// The `Records` row was deleted.
    Deleted(DeletedRowData, State<'conn, DeletedRecordKey>),
    /// The `Records` row does not exist and the `NullRecords` row exists.
    Null(State<'conn, NullRecordRow>),
    /// The `Records` and `NullRecords` rows do not exist.
    Unknown(State<'conn, Missing>),
}

/// A representation of the database state beginning with an arbitrary [`RemoteId`].
#[derive(Debug)]
pub enum ExistsOrUnknown<'conn> {
    /// The `Records` row exists.
    Entry(EntryRowData, State<'conn, EntryRecordKey>),
    /// The `Records` row was deleted.
    Deleted(DeletedRowData, State<'conn, DeletedRecordKey>),
    /// The `Records` and `NullRecords` rows do not exist.
    Unknown(State<'conn, Missing>),
}

impl<'conn> RemoteIdState<'conn> {
    #[inline]
    pub fn delete_null(self) -> Result<ExistsOrUnknown<'conn>, rusqlite::Error> {
        Ok(match self {
            RemoteIdState::Entry(data, state) => ExistsOrUnknown::Entry(data, state),
            RemoteIdState::Deleted(data, state) => ExistsOrUnknown::Deleted(data, state),
            RemoteIdState::Null(state) => ExistsOrUnknown::Unknown(state.delete()?),
            RemoteIdState::Unknown(state) => ExistsOrUnknown::Unknown(state),
        })
    }

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
                match State::init(tx, RecordKey(row_id)).determine()? {
                    EntryOrDeletedRow::Entry(entry_row_data, state) => {
                        RemoteIdState::Entry(entry_row_data, state)
                    }
                    EntryOrDeletedRow::Deleted(deleted_row_data, state) => {
                        RemoteIdState::Deleted(deleted_row_data, state)
                    }
                }
            }
            None => match get_null_row_id(&tx, remote_id)? {
                Some(row_id) => {
                    debug!(
                        "Beginning new transaction for row '{row_id}' in the `NullRecords` table."
                    );
                    Self::Null(State::init(tx, NullRecordRow(row_id)))
                }
                None => {
                    debug!("Beginning new transaction for unknown remote id.");
                    Self::Unknown(State::init(tx, Missing {}))
                }
            },
        })
    }
}
