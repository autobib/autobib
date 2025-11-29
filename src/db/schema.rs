//! This folder contains the current database schemas as well as all previous database schemas.
//!
macro_rules! schema {
    ($name:ident, $desc:expr) => {
        #[doc = concat!($desc, ".")]
        ///
        /// The database schema contents:
        /// ```sql
        #[doc = include_str!(concat!("schema/", stringify!($name), ".sql"))]
        ///```
        pub const fn $name() -> &'static str {
            include_str!(concat!("schema/", stringify!($name), ".sql"))
        }
    };
}

schema!(identifiers, "The lookup table for identifiers.");

schema!(records, "The table which stores record data.");

schema!(null_records, "The table which caches null records.");

schema!(create_indices, "Create indices for the tables.");
