//! # SQL statements
//! This module contains all of the SQL statements used by [`RecordDatabase`](`super::RecordDatabase`).
//! The statements are included in the documentation for the corresponding functions.

/// A convenience macro for generating the function and the corresponding documentation.
macro_rules! sql {
    ($name:ident, $desc:expr) => {
        #[doc = concat!($desc, ".")]
        ///
        /// Returns the following statement as a string:
        /// ```sql
        #[doc = include_str!(concat!("sql/", stringify!($name), ".sql"))]
        ///```
        pub const fn $name() -> &'static str {
            include_str!(concat!("sql/", stringify!($name), ".sql"))
        }
    };
}

sql!(init_records, "Create the records table");

sql!(init_citation_keys, "Create the citation keys table");

sql!(init_null_records, "Create the null records table");

sql!(set_wal, "Set write-ahead log");

sql!(optimize, "Optimize the database");

sql!(get_table_schema, "Get the table schema");

sql!(get_cached_data, "Get cached record data");

sql!(set_cached_data, "Set cached record data");

sql!(update_cached_data, "Update cached record data");

sql!(get_cached_null, "Get cached null data");

sql!(set_cached_null, "Set cached null data");

sql!(rename_citation_key, "Rename a citation key");

sql!(get_record_key, "Get a record key");

sql!(delete_citation_key, "Delete a citation key");

sql!(
    set_citation_key_overwrite,
    "Set a citation key, replacing if one already exists"
);

sql!(
    set_citation_key_fail,
    "Set a citation key, failing if one already exists"
);

sql!(
    set_citation_key_ignore,
    "Set a citation key, overwriting if one already exists"
);
