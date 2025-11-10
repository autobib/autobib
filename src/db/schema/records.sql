CREATE TABLE Records (
    key INTEGER PRIMARY KEY,
    record_id TEXT NOT NULL,
    data BLOB NOT NULL,
    modified TEXT NOT NULL,
    parent INTEGER,
    children BLOB NOT NULL DEFAULT x'',
    active INTEGER NOT NULL DEFAULT 1
) STRICT
