CREATE TABLE Records (
    key INTEGER PRIMARY KEY,
    record_id TEXT NOT NULL UNIQUE,
    data BLOB NOT NULL,
    modified TEXT NOT NULL
) STRICT
