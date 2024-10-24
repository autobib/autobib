mod missing;
mod null;
mod record;
mod transaction;

use log::debug;
use rusqlite::Transaction;

pub use self::{missing::*, null::*, record::*, transaction::DatabaseState};
use super::{get_null_row_id, get_row_id};
use crate::{error::RecordError, Alias, AliasOrRemoteId, RecordId, RemoteId};

/// A representation of the database state beginning with an arbitrary [`RecordId`].
#[derive(Debug)]
pub enum RecordIdState<'conn> {
    /// The `Records` row exists.
    Existent(RecordId, RecordRow<'conn>),
    /// The `Records` row does not exist and the `NullRecords` row exists.
    NullRemoteId(RemoteId, NullRecordRow<'conn>),
    /// The `Records` and `NullRecords` rows do not exist.
    UnknownRemoteId(RemoteId, MissingRow<'conn>),
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
        match get_row_id(&tx, &record_id)? {
            Some(row_id) => {
                debug!("Beginning new transaction for row '{row_id}' in the `Records` table.");
                Ok(RecordIdState::Existent(
                    record_id,
                    RecordRow::new(tx, row_id),
                ))
            }
            None => match record_id.resolve() {
                Ok(AliasOrRemoteId::Alias(alias)) => {
                    tx.commit()?;
                    Ok(RecordIdState::UndefinedAlias(alias))
                }
                Ok(AliasOrRemoteId::RemoteId(remote_id)) => match get_null_row_id(&tx, &remote_id)?
                {
                    Some(row_id) => {
                        debug!("Beginning new transaction for row '{row_id}' in the `NullRecords` table.");
                        Ok(RecordIdState::NullRemoteId(
                            remote_id,
                            NullRecordRow::new(tx, row_id),
                        ))
                    }
                    None => {
                        debug!("Beginning new transaction for unknown remote id.");
                        Ok(RecordIdState::UnknownRemoteId(
                            remote_id,
                            MissingRow::new(tx),
                        ))
                    }
                },
                Err(record_error) => {
                    tx.commit()?;
                    Ok(RecordIdState::InvalidRemoteId(record_error))
                }
            },
        }
    }
}

/// A representation of the database state beginning with an arbitrary [`RemoteId`].
#[derive(Debug)]
pub enum RemoteIdState<'conn> {
    /// The `Records` row exists.
    Existent(RecordRow<'conn>),
    /// The `Records` row does not exist and the `NullRecords` row exists.
    Null(NullRecordRow<'conn>),
    /// The `Records` and `NullRecords` rows do not exist.
    Unknown(MissingRow<'conn>),
}

impl<'conn> RemoteIdState<'conn> {
    /// Determine the current state of the database, as corresponds to the provided remote record
    /// identifier.
    #[inline]
    pub fn determine(
        tx: Transaction<'conn>,
        remote_id: &RemoteId,
    ) -> Result<Self, rusqlite::Error> {
        match get_row_id(&tx, remote_id)? {
            Some(row_id) => {
                debug!("Beginning new transaction for row '{row_id}' in the `Records` table.");
                Ok(Self::Existent(RecordRow::new(tx, row_id)))
            }
            None => {
                match get_null_row_id(&tx, remote_id)? {
                    Some(row_id) => {
                        debug!("Beginning new transaction for row '{row_id}' in the `NullRecords` table.");
                        Ok(Self::Null(NullRecordRow::new(tx, row_id)))
                    }
                    None => {
                        debug!("Beginning new transaction for unknown remote id.");
                        Ok(Self::Unknown(MissingRow::new(tx)))
                    }
                }
            }
        }
    }
}
