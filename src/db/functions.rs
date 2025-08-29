use rusqlite::{Connection, functions::FunctionFlags};

/// The available application functions.
///
/// Function descriptions:
///
/// - [`Regexp`](AppFunction::Regexp): `regexp(re: TEXT, value: TEXT) -> BOOL` returns if `value`
///   matches the regex defined in `re`
/// - [`ContainsField`](AppFunction::ContainsField): `contains_field(field: TEXT, data: BLOB) -> BOOL`
///   returns if `data` (interpreted as record data) contains the provided field
/// - [`GetField`](AppFunction::GetField): `get_field(field: TEXT, data: BLOB) -> TEXT or NULL`
///   returns the field value if it exists, or null.
#[derive(Debug)]
pub enum AppFunction {
    /// Checks if a string matches a given regex.
    Regexp,
    /// Checks if record data contains a field.
    ContainsField,
    /// Returns the field value from the record data.
    GetField,
}

impl AppFunction {
    /// The name of the function for use in SQL queries.
    pub fn name(&self) -> &'static str {
        match self {
            AppFunction::Regexp => "regexp",
            AppFunction::ContainsField => "contains_field",
            AppFunction::GetField => "get_field",
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
    use crate::entry::{EntryData, RawRecordData};

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

                RawRecordData::from_byte_repr_unchecked(data).contains_field(field_name)
            };

            Ok(is_match)
        },
    )
}

/// Register `get_field` callback.
fn add_get_field_function(conn: &Connection) -> Result<(), rusqlite::Error> {
    use crate::entry::{BorrowedEntryData, RawRecordData};

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

                RawRecordData::from_byte_repr_unchecked(data).get_field_borrowed(field_name)
            };

            // this has to be 'static
            Ok(field_value.map(ToOwned::to_owned))
        },
    )
}
