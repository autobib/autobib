# Architecture
## SQLite database format

### `Records` table
This table has schema
 ```
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
```
CREATE TABLE CitationKeys (
    name TEXT NOT NULL PRIMARY KEY,
    record_key INTEGER,
    CONSTRAINT foreign_record_key
        FOREIGN KEY (record_key)
        REFERENCES Records(key)
        ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
```
This table stores the keys which are used to lookup records.
Canonical ids, reference ids, and aliases are all stored in this table.

### `Changelog` table
This table has schema
 ```
 CREATE TABLE Changelog (
     record_id TEXT NOT NULL,
     data BLOB NOT NULL,
     modified TEXT NOT NULL
 ) STRICT;
 ```
 Whenever a row in the `Records` table is modified or deleted, the row is copied into the `Changelog` table as a backup.

### `NullRecords` table
This table has schema
```
 CREATE TABLE NullRecords (
     record_id TEXT NOT NULL PRIMARY KEY,
     attempted TEXT NOT NULL
 ) STRICT;
 ```
 This table records the failed lookups if a provided record is invalid.

## Internal binary data format
TODO: write

## Lookup flow
Given a CLI call of the form
```
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
