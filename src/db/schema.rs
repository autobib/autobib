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

schema!(changelog, "The citation keys table.");

schema!(citation_keys, "The citation keys table.");

schema!(records, "The citation keys table.");

schema!(null_records, "The citation keys table.");
