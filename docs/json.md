# JSON output schemas

Some Autobib commands can output JSON.
This page documents the corresponding schemas.

- `autobib source --json`: [`$source`](#source-autobib-source-output)
- The `%json` meta key in templates: [`$record_entry`](record-entry-record-of-type-entry)
- `autobib info -r all`: [`$info`](#info-autobib-info-output)

## Schemas

In the below pseudo-schemas, the syntax `$...` is used to refer to another schema.
The basic schemas are:

- `null`: JSON null
- `$string`: a JSON string
- `$boolean`: a JSON boolean
- `$hex`: a lowercase hexadecimal string
- `$date_time`: an [ISO 8601](https://en.wikipedia.org/wiki/ISO_8601) datetime in local timezone, like `YYYY-MM-DDTHH:MM:SS.SSSSSS+HH:MM`

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

Schema: [`record_deleted.schema.json`](/docs/schema/record_deleted.schema.json).

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

The schema is defined precisely in [`record_entry.schema.json`](/docs/schema/record_entry.schema.json).

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
  "modified": "$date_time
}
```

## `$source`: Autobib source output

Schema: [`source.schema.json`](/docs/schema/source.schema.json).

The command `autobib source --json` outputs a dictionary mapping keys to record data.
The schema is defined in [`source.schema.json`](/docs/schema/source.schema.json).

Each key is a string corresponding to a citation key found in the input and the values are JSON objects containing the entry data.
The JSON looks like:
```json
{
  "$string": "$entry_record"
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

The *info* schema contains all information which can be obtained from a key which matches  a record in the database.
This contains information about the key itself, the revision (used to identify the exact version in the revision tree), and the record, which may be an entry, deleted, or void.
The JSON looks like:
```json
{
  "key": {
    "original": "$string",
    "is_valid_bibtex: "$boolean",
    "preferred": "$string | null",
    "equivalent": ["$string"]
  },
  "revision": "$hex",
  "record": "$record_entry | $record_deleted | $record_void"
```
The `equivalent` array contain other keys which map to the same record.
The `preferred` string is the preferred key, as determined by your configuration.
