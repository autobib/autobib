use log::debug;
use rusqlite::{types::ValueRef, Transaction};

use super::{get_row_id, sql};
use crate::{error::ValidationError, RawRecordData, RecordId, RemoteId};

pub struct DatabaseValidator<'conn> {
    pub tx: Transaction<'conn>,
}

impl<'conn> DatabaseValidator<'conn> {
    pub fn commit(self) -> Result<(), ValidationError> {
        Ok(self.tx.commit()?)
    }

    pub fn record_indexing(&self) -> Result<(), ValidationError> {
        let mut retriever = self.tx.prepare(sql::get_all_record_data())?;
        let mut rows = retriever.query([])?;

        // rows does not implement Iterator
        while let Some(row) = rows.next()? {
            // first verify that we actually get a proper canonical id
            let contents: String = row.get("record_id")?;
            let canonical_id: RemoteId = match RecordId::from(contents.as_ref()).try_into() {
                Ok(remote_id) => remote_id,
                Err(_) => {
                    return Err(ValidationError::ConsistencyError(format!(
                        "Record row contains record id '{contents}' which is not a valid canonical id"
                    )));
                }
            };

            // now, check that it is actually valid
            if get_row_id(&self.tx, &canonical_id)?.is_none() {
                return Err(ValidationError::DanglingRecord(contents));
            }
        }
        Ok(())
    }

    pub fn consistency(&self) -> Result<(), ValidationError> {
        let mut errors: Option<String> = None;

        debug!("Checking foreign key constraints");
        self.tx.pragma_query(None, "foreign_key_check", |row| {
            let msg: String = row.get(0)?;
            let error_msg = errors.get_or_insert_with(String::new);
            error_msg.push_str("\nForeign key constraint error: ");
            error_msg.push_str(&msg);
            Ok(())
        })?;

        debug!("Checking database integrity");
        self.tx.pragma_query(None, "integrity_check", |row| {
            if !matches!(row.get_ref(0)?, ValueRef::Text(b"ok")) {
                let source_table: String = row.get(0)?;
                let source_row_id: String = row.get(1)?;
                let target_table: String = row.get(2)?;
                let target_row_id: String = row.get(3)?;

                let contents = format!("Row '{source_row_id}' in table '{source_table}' has invalid reference to row '{target_row_id}' in '{target_table}'");

                let error_msg = errors.get_or_insert_with(String::new);
                error_msg.push_str("\nConsistency error: ");
                error_msg.push_str(&contents);
            }
            Ok(())
        })?;

        if let Some(message) = errors {
            Err(ValidationError::ConsistencyError(message))
        } else {
            Ok(())
        }
    }

    /// Validate binary data inside a transaction.
    pub fn record_data(&self) -> Result<(), ValidationError> {
        debug!("Validating binary record data");
        let mut retriever = self.tx.prepare(sql::get_all_record_data())?;
        let mut rows = retriever.query([])?;

        // rows does not implement Iterator
        while let Some(row) = rows.next()? {
            if let Err(err) = RawRecordData::from_byte_repr(row.get("data")?) {
                return Err(ValidationError::MalformedRecordData(
                    row.get("record_id")?,
                    err,
                ));
            }
        }
        Ok(())
    }
}
