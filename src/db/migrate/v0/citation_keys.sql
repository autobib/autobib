CREATE TABLE CitationKeys (
    name TEXT NOT NULL PRIMARY KEY,
    record_key INTEGER,
    CONSTRAINT foreign_record_key
        FOREIGN KEY (record_key)
        REFERENCES Records(key)
        ON DELETE CASCADE
) STRICT, WITHOUT ROWID
