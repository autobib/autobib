# Architecture

## SQLite database format

The Autobib command-line tool stores data locally in a [SQLite](https://sqlite.org/) database, located (in order of priority)

- at the location specified by the `--database` command line option
- as set by the `$AUTOBIB_DATABASE_PATH` environment variable
- by default at `$XDG_CONFIG_HOME/autobib/records.db`

The goal is this section is to give a full, detailed description of the database format.

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
     record_id TEXT NOT NULL UNIQUE,
     data BLOB NOT NULL,
     modified TEXT NOT NULL
 ) STRICT;
 ```

 This table stores the record data, and the associated canonical id, as well as the last modified time.

### `CitationKeys` table

This table has schema

```sql
CREATE TABLE CitationKeys (
    name TEXT NOT NULL PRIMARY KEY,
    record_key INTEGER,
    CONSTRAINT foreign_record_key
        FOREIGN KEY (record_key)
        REFERENCES Records(key)
        ON UPDATE CASCADE ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
```

This table stores the keys which are used to lookup records.
Canonical ids, reference ids, and aliases are all stored in this table.

### `Changelog` table

This table has schema

 ```sql
 CREATE TABLE Changelog (
     record_id TEXT NOT NULL,
     data BLOB NOT NULL,
     modified TEXT NOT NULL
 ) STRICT;
 ```

Whenever a row in the `Records` table is modified or deleted, the row is copied into the `Changelog` table as a backup.

### `NullRecords` table

This table has schema

```sql
 CREATE TABLE NullRecords (
     record_id TEXT NOT NULL PRIMARY KEY,
     attempted TEXT NOT NULL
 ) STRICT;
 ```

This table records the failed lookups if a provided record is invalid.

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

where `key_len` is the length of the first `key` segment, and the `value_len` is
the length of the `value` segment. Necessarily, `key` and `value` have lengths at
most `u8::MAX` and `u16::MAX` respectively.

`value_len` is encoded in little endian format.

The `DATA[i]` are sorted by `key` and each `key` and `entry_type` must be ASCII lowercase. The
`entry_type` can be any valid UTF-8.

## Lookup flow

Given a CLI call of the form

```sh
autobib <source>:<sub_id>
```

perform the following lookup:

```txt
    ┏━━━━━━━━━━━━━━━┓
    ┃INPUT: RecordId┃
    ┗━━━━━━━━━━━━━━━┛
            │                                                     ┌ ─ ─ ─RETURN VALUES─ ─ ─ ─
            ▽                                                                                │
╔══════════════════════╗   ┏━━━┓                                  │  ┌───────────┐
║in CitationKeys table?║──▶┃YES┃────────────┌───────────────────────▷│ Ok(Entry) │           │
╚══════════════════════╝   ┗━━━┛            │                     │  └───────────┘
            │                               │                                                │
            ▼                               │                     │
          ┏━━━┓                             │                                                │
          ┃NO ┃            ┏━━━━━┓          │                     │  ┌────────────────┐
          ┗━━━┛        ┌──▶┃Alias┃──────────│───────────────────────▷│ Err(NullAlias) │      │
            │          │   ┗━━━━━┛          │                     │  └────────────────┘
            ▽          │   ┏━━━━━━━━┓       │                                                │
     ╔═════════════╗   │   ┃invalid ┃       │                     │  ┌──────────────────┐
     ║valid remote ║   ├──▶┃RecordId┃───────│───────────────────────▷│ Err(BadRecordId) │    │
     ║id or alias? ║───┘   ┗━━━━━━━━┛       │                     │  └──────────────────┘
     ╚═════════════╝                 ┌────────────┐  ┌─────────┐                             │
            │                        │add Context │  │  cache  │  │  ┌─────────────────┐
            ▼                        │     to     │  │ Context │────▷│ Err(NullRemote) │     │
       ┏━━━━━━━━━┓                   │CitationKeys│  │ as Null │  │  └─────────────────┘
       ┃RemoteId ┃                   └────────────┘  └─────────┘                             │
       ┗━━━━━━━━━┛                          △             △       │
 ─ ─ ─ ─ ─ ─│─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─│─ ─ ─ ─ ─ ─ ─│─ ─ ─                             │
│           │           ┏━━━┓       ┏━━━┓   │             │     │ │  ┌── ─── ─── ─── ─── ─┐
            ┌───────────┃NO ┃◀──┬──▶┃YES┃───┘    ┌────────┘          │ Err(DatabaseError) │  │
│           ▽           ┗━━━┛   │   ┗━━━┛   │    │              │ │  └ ─── ─── ─── ─── ───
     ╔═════════════╗            │           │    │                                           │
│    ║cached Null? ║    ╔═══════════════╗   │    │              │ │  ┌── ─── ─── ─── ─── ─┐
     ╚═════════════╝    ║in CitationKeys║   │    │  ┌────────┐       │ Err(NetworkError)  │  │
│           │           ║    table?     ║   │    │  │ insert │  │ │  └ ─── ─── ─── ─── ───
            ├──────┐    ╚═══════════════╝   └───────│Entry to│                               │
│           ▼      ▼            △                │  │Records │  │ └ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─
          ┏━━━┓  ┏━━━┓          │                │  └────────┘
│         ┃NO ┃  ┃YES┃──────────│────────────────┘       △      │
          ┗━━━┛  ┗━━━┛          │                │       │          MAGENTA = Database
│           │              ┏━━━━━━━━━┓        ┏━━━━┓  ┏━━━━━┓   │   Operation
            ▽              ┃RemoteId ┃        ┃Null┃  ┃Entry┃
│  ┌─────────────────┐     ┗━━━━━━━━━┛        ┗━━━━┛  ┗━━━━━┛   │   BLUE = Network Operation
   │push RemoteId to │          ▲             ▲    ▲     ▲
│  │     Context     │          └────────────┬┘    └────┬┘      │   Special errors can occur
   └─────────────────┘                       │          │           within relevant nodes.
│           │            ┏━━━━━━━━━━━┓   ╔═══════╗  ╔═══════╗   │
            ▽         ┌─▶┃ReferenceId┃──▷║convert║  ║lookup ║
│    ╔═════════════╗  │  ┗━━━━━━━━━━━┛   ╚═══════╝  ╚═══════╝   │
     ║canonical or ║  │  ┏━━━━━━━━━━━┓                  △
│    ║ reference?  ║──┴─▶┃CanonicalId┃──────────────────┘       │
     ╚═════════════╝     ┗━━━━━━━━━━━┛
│                                                               │
 ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ REMOTE RESOLVE LOOP ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ 
```
