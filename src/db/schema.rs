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

schema!(keys, "The lookup table for identifiers.");

schema!(records, "The table which stores record data.");

schema!(null_records, "The table which caches null records.");

/// The indices created when initializing a database.
pub const INDICES: &[(&str, &str)] = &[
    (
        "records_parent_rev",
        "CREATE INDEX records_parent_rev ON Records(parent_rev)",
    ),
    (
        "records_canonical",
        "CREATE INDEX records_canonical ON Records(canonical)",
    ),
    (
        "records_modified",
        "CREATE INDEX records_modified ON Records(modified)",
    ),
    (
        "keys_record_rev",
        "CREATE INDEX keys_record_rev ON Keys(record_rev)",
    ),
];

/// The views created when initializing a database.
pub const VIEWS: &[(&str, &str)] = &[(
    "ActiveRecords",
    "CREATE VIEW ActiveRecords AS
SELECT canonical, modified, data as entry_data
FROM Records
WHERE
  rev in (SELECT record_rev FROM Keys)
  AND variant = 0",
)];
