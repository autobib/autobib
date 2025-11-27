CREATE TABLE CitationKeys (
    name TEXT NOT NULL PRIMARY KEY,
    record_key INTEGER NOT NULL,
    CONSTRAINT foreign_record_key
        FOREIGN KEY (record_key)
        REFERENCES Records(key)
        ON UPDATE CASCADE ON DELETE CASCADE
) STRICT, WITHOUT ROWID
