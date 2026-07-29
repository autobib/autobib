use autobib_entry::{
    Archive,
    data::{EntryData, archive},
    v0, v1,
};
use rusqlite::{Connection, functions::FunctionFlags};

/// The available application functions.
#[derive(Debug)]
pub enum AppFunction {
    /// `regexp(re: TEXT, value: TEXT) -> BOOL` returns if `value` matches the regex defined in `re`
    Regexp,
    /// `contains_field(field: TEXT, data: BLOB) -> BOOL` returns if the record data contains the provided field
    ContainsField,
    ///`get_field(field: TEXT, data: BLOB) -> TEXT or NULL` returns the field value if it exists, or null.
    GetField,
    ///`update_entry_data(data: BLOB) -> BLOB` accepts entry data in the `v0` format and returns
    ///entry data in the `v1` format.
    UpdateEntryData,
}

impl AppFunction {
    /// The name of the function for use in SQL queries.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Regexp => "regexp",
            Self::ContainsField => "contains_field",
            Self::GetField => "get_field",
            Self::UpdateEntryData => "update_entry_data",
        }
    }
}

pub fn register_application_function(
    conn: &Connection,
    fun: AppFunction,
) -> Result<(), rusqlite::Error> {
    match fun {
        AppFunction::Regexp => add_regexp_function(conn),
        AppFunction::ContainsField => add_contains_field_function(conn),
        AppFunction::GetField => add_get_field_function(conn),
        AppFunction::UpdateEntryData => add_update_entry_data_function(conn),
    }
}

/// Register `regexp` callback.
fn add_regexp_function(conn: &Connection) -> Result<(), rusqlite::Error> {
    use regex::Regex;
    use std::sync::Arc;

    conn.create_scalar_function(
        AppFunction::Regexp.name(),
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        move |ctx| {
            assert_eq!(ctx.len(), 2, "called with unexpected number of arguments");
            let regexp: Arc<Regex> = ctx.get_or_create_aux(
                0,
                |vr| -> Result<_, Box<dyn std::error::Error + Send + Sync + 'static>> {
                    Ok(Regex::new(vr.as_str()?)?)
                },
            )?;
            let is_match = {
                let text = ctx
                    .get_raw(1)
                    .as_str()
                    .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?;

                regexp.is_match(text)
            };

            Ok(is_match)
        },
    )
}

/// Register `contains_field` callback.
fn add_contains_field_function(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.create_scalar_function(
        AppFunction::ContainsField.name(),
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        move |ctx| {
            assert_eq!(ctx.len(), 2, "called with unexpected number of arguments");
            let field_name = ctx
                .get_raw(1)
                .as_str()
                .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?;

            let is_match = {
                let data = ctx
                    .get_raw(0)
                    .as_blob()
                    .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?;

                v0::ArchivedEntryData::access(data)
                    .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?
                    .contains_field(field_name)
            };

            Ok(is_match)
        },
    )
}

/// Register `get_field` callback.
fn add_get_field_function(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.create_scalar_function(
        AppFunction::GetField.name(),
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        move |ctx| {
            assert_eq!(ctx.len(), 2, "called with unexpected number of arguments");
            let field_name = ctx
                .get_raw(1)
                .as_str()
                .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?;

            let field_value = {
                let data = ctx
                    .get_raw(0)
                    .as_blob()
                    .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?;

                v0::ArchivedEntryData::access(data)
                    .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?
                    .get_field_str(field_name)
            };

            // this has to be 'static
            Ok(field_value.map(ToOwned::to_owned))
        },
    )
}

/// Register `update_entry_data` callback.
fn add_update_entry_data_function(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.create_scalar_function(
        AppFunction::UpdateEntryData.name(),
        1,
        FunctionFlags::SQLITE_DETERMINISTIC,
        move |ctx| {
            assert_eq!(ctx.len(), 1, "called with unexpected number of arguments");
            let data = ctx
                .get_raw(0)
                .as_blob()
                .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?;

            let new_data = match data.first() {
                Some(0) => {
                    let v0_data = v0::ArchivedEntryData::access(data)
                        .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?;
                    archive::<v1::ArchivedEntryData, _>(&v0_data)
                }
                Some(1) => data.into(),
                _ => {
                    return Err(rusqlite::Error::UserFunctionError(
                        autobib_entry::error::AccessError::Unrecognized.into(),
                    ));
                }
            };

            Ok(new_data)
        },
    )
}
