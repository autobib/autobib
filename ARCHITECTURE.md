# Architecture

This documentation is for **database version 2**.
Please see older copies of this file for different database versions.

## SQLite database format

The Autobib command-line tool stores data locally in a [SQLite](https://sqlite.org/) database, located (in order of priority)

- at the location specified by the `--database` command line option
- as set by the `$AUTOBIB_DATABASE_PATH` environment variable
- by default at `$XDG_CONFIG_HOME/autobib/records.db`

The goal is this section is to give a full, detailed description of the database format in order to read data from the database without using the Autobib program.

### Application identifier and database version

An Autobib database can be identified by the application identifier.
This is tracked in the SQLite `application_id` field, and can be read with
```sql
PRAGMA application_id;
```
The application id is hex `16611f2f` or decimal `375463727`.
This is the `sha256` hash of the string `Autobib`:
```sh
echo -n "Autobib" | sha256 | head -c 8
```
The database version is tracked in the SQLite `user_version` tag, and can be read with
```sql
PRAGMA user_version;
```

### `Records` table

This table has schema
```sql
CREATE TABLE Records (
    key INTEGER PRIMARY KEY,
    record_id TEXT NOT NULL,
    data BLOB NOT NULL,
    modified TEXT NOT NULL,
    variant INTEGER NOT NULL DEFAULT 0,
    parent_key INTEGER REFERENCES Records(key)
        ON UPDATE RESTRICT
        ON DELETE SET NULL
) STRICT;
```
This table stores the record data, and the associated canonical id, as well as the last modified time.

The `data` column contains the raw data associated with the record, with interpretation based
on the `variant`.

- `variant = 0`: This is a regular entry which contains some data.
  The data is encoded according to the rules [documented below](#internal-binary-data-format).
- `variant = 1`: This is a deleted entry.
  Either `data` is a non-empty UTF-8 string (indicating a replacement record), or it is empty.
  The replacement record may not exist in the database, though it is checked to exist at the time of creation.
- `variant = 2`: The special void marker.
  The value in `data` is ignored and should be empty.

### `Identifiers` table

This table has schema
```sql
CREATE TABLE Identifiers (
    name TEXT NOT NULL PRIMARY KEY,
    record_key INTEGER NOT NULL References Records(key)
        ON UPDATE RESTRICT
        ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
```
This is a lookup table mapping identifiers to record keys.

### `NullRecords` table

This table has schema
```sql
CREATE TABLE NullRecords (
    record_id TEXT NOT NULL PRIMARY KEY,
    attempted TEXT NOT NULL
) STRICT;
```
This is a cache table for failed lookup if a provided record is invalid.

### Database invariants

The following invariants must be upheld at all times.

1. The `parent_key` row indicates a directed edge leading from a given row to its *parent* row.
   The set of rows for a given value of `record_id` must form exactly one tree.
2. The `modified` column must be sorted in descending order down the tree: that is, each parent must have `modified` time which is greater than the `modified` time of the child node.
3. If a void node exists, its `parent_key` must be null.
4. The modification time of the void node must be exactly `-262143-01-01 00:00:00+00:00`.
5. A row in the 'Records' table with a key that is present in the `Identifiers` table is called *active*.
   Exactly one row per `record_id`-tree must be active.

## Internal binary data format

We use a custom internal binary format to represent the data associated with each bibTex entry.

The data is stored as
```txt
VERSION(u8), DATA(..)
```
The first byte is the version.
Depending on the version, the format of `DATA` is as follows.

### Version 0

The data is stored as a sequence of blocks.
```txt
TYPE, DATA[0], DATA[1], ..
```
The `TYPE` consists of
```txt
[entry_type_len: u8, entry_type: [u8..]]
```
Here, `entry_type_len` is the length of `entry_type`, which has length at most `u8::MAX`.
Then, each block `DATA` is of the form
```txt
[key_len: u8, value_len: u16, key: [u8..], value: [u8..]]
```
where `key_len` is the length of the first `key` segment, and the `value_len` is the length of the `value` segment. Necessarily, `key` and `value` have lengths at most `u8::MAX` and `u16::MAX` respectively.

`value_len` is encoded in little endian format.

The `DATA[i]` are sorted by `key` and each `key` and `entry_type` must be ASCII lowercase.
The `entry_type` can be any valid UTF-8.
