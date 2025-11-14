CREATE TABLE Records (
    key INTEGER PRIMARY KEY,
    record_id TEXT NOT NULL,
    data BLOB NOT NULL,
    modified TEXT NOT NULL,
    variant INTEGER NOT NULL DEFAULT 0,
    parent_key INTEGER,
    children BLOB NOT NULL DEFAULT x'',
    CONSTRAINT foreign_parent_key
        FOREIGN KEY (parent_key)
        REFERENCES Records(key)
        ON UPDATE CASCADE ON DELETE CASCADE
) STRICT
