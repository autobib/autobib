# JSON output schemas

Some Autobib commands can output JSON.
This page documents the corresponding schemas.

- `autobib source --json`: [`$source`](#source-autobib-source-output)
- The `%json` meta key in templates: [`$record_entry`](#record_entry-record-of-type-entry)
- `autobib info --json`: [`$info`](#info-autobib-info-output) for the `all` report type

## Schemas

In the below pseudo-schemas, the syntax `$...` is used to refer to another schema.
The basic schemas are:

- `null`: JSON null
- `$string`: a JSON string
- `$boolean`: a JSON boolean
- `$hex`: a lowercase hexadecimal string
- `$date_time`: a date-time in the local time zone, represented as per [IETF RFC 3339](https://datatracker.ietf.org/doc/html/rfc3339#section-5.6) (it often looks like `YYYY-MM-DDTHH:MM:SS.SSSSSSSSS+HH:MM`)

The `|` modifier is used to represent an alternative.
For example, `$string | null` is either a string or a JSON null.

Each section also contains links to [JSON schemas](https://json-schema.org/) for precise machine-readable formats.
Note that the machine-readable schemas are more precise and also constrain the values that some of the strings may take.

### `$data`: Data

Schema: [`data.schema.json`](/docs/schema/data.schema.json).

*Entry data* is the data which is present in a BibTeX entry, not including the citation key itself.
This includes the *entry type* (such as `article`) and the `fields` (with keys such `title` and string values).
The data looks like:
```json
{
  "entry_type": "$string",
  "fields": {
    "$string": "$string"
  }
}
```
The field keys will be sorted in alphabetical order.

For example,
```json
{
  "entry_type": "article",
  "fields": {
    "author": "John Doe",
    "title": "A book",
    "year": "1939"
  }
}
```

### `$record_entry`: Record (of type entry)

Schema: [`record_entry.schema.json`](/docs/schema/record_entry.schema.json).

An *entry record* contains all of the information associated with a record which exists in the database and is not deleted or voided.
This includes the entry data, as well as the canonical identifier and a modification timestamp.
The JSON looks like:
```json
{
  "data": "$data",
  "canonical": "$string",
  "modified": "$date_time"
}
```

### `$record_deleted`: Record (of type deleted)

Schema: [`record_deleted.schema.json`](/docs/schema/record_deleted.schema.json).

A *deleted record* contains all of the information associated with a record which has been deleted.
This includes a replacement key (which may be `null`), as well as the canonical identifier and a modification timestamp.
The JSON looks like:
```json
{
  "replacement": "$string | null",
  "canonical": "$string",
  "modified": "$date_time"
}
```

### `$record_void`: Record (of type void)

Schema: [`record_void.schema.json`](/docs/schema/record_void.schema.json).

A *void record* is a special record essentially equivalent to a record which is not present in the database, but does contain a small amount of metadata (the canonical identifier and the modification timestamp).
The JSON looks like:
```json
{
  "canonical": "$string",
  "modified": "$date_time"
}
```

## `$source`: Autobib source output

Schema: [`source.schema.json`](/docs/schema/source.schema.json).

The command `autobib source --json` outputs a dictionary mapping keys to entry data.

Each key is a string corresponding to a citation key found in the input, and each value is the [entry data](#data-data) associated with that key.
Note that the values are bare entry data: unlike [`$record_entry`](#record_entry-record-of-type-entry), they do not contain the canonical identifier or the modification timestamp.
The JSON looks like:
```json
{
  "$string": "$data"
}
```
For example, the output might look something like
```json
{
  "key1": {
    "entry_type": "article",
    "fields": {
      "author": "Alice",
      "title": "Alice's article",
      "year": "1900"
    }
  },
  "key2": {
    "entry_type": "book",
    "fields": {
      "author": "Bob",
      "title": "Bob's book",
      "pagetotal": "192"
    }
  }
}
```
There may be duplicate values: each key corresponding to a valid entry will occur exactly once in the output data.
By default, `autobib source` prints warnings if there are undefined keys.

## `$info`: Autobib info output

Schema: [`info.schema.json`](/docs/schema/info.schema.json).

The `$info` schema describes the output of `autobib info --json` with report type `--report all`.
It contains all information which can be obtained from a key which matches a record in the database.
This contains information about the key itself, the revision (used to identify the exact version in the revision tree), and the record, which may be an entry, deleted, or void.
The JSON looks like:
```json
{
  "key": {
    "original": "$string",
    "is_valid_bibtex": "$boolean",
    "user_preferred": "$string | null",
    "equivalent": ["$string"]
  },
  "revision": "$hex",
  "record": "$record_entry | $record_deleted | $record_void"
}
```
The `equivalent` array contain other keys which map to the same record.
The `user_preferred` value is the preferred key, as determined by your configuration, or `null` if no keys match.

### Other report types

The other report types also accept `--json`.

| Report type  | JSON output   |
| ------------ | ------------- |
| `all`        | `$info`       |
| `canonical`  | `$string`     |
| `bibtex`     | `$boolean`    |
| `equivalent` | `["$string"]` |
| `preferred`  | `$string`     |
| `modified`   | `$date_time`  |
| `revision`   | `$hex`        |

> [!NOTE]
> The `preferred` report type falls back to the canonical identifier if no keys match.
> Unlike the `user_preferred` field of `$info`, it is never `null`.
