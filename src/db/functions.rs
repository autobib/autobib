use std::sync::Arc;

use regex::Regex;
use rusqlite::{Connection, functions::FunctionFlags};

/// The available application functions.
#[derive(Debug)]
pub enum AppFunction {
    /// Defines a `regexp` function which allows checking for regex matches.
    Regexp,
}

impl AppFunction {
    /// The name of the function for use in SQL queries.
    pub fn name(&self) -> &'static str {
        match self {
            AppFunction::Regexp => "regexp",
        }
    }
}

pub fn register_application_function(
    conn: &Connection,
    fun: AppFunction,
) -> Result<(), rusqlite::Error> {
    match fun {
        AppFunction::Regexp => add_regexp_function(conn),
    }
}

/// Register a regex callback for use by the SQLITE `regexp` command.
fn add_regexp_function(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.create_scalar_function(
        "regexp",
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
