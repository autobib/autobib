# The Autobib data model

## Identifiers

Any way to refer to record data stored in your database is called an **identifier**.
There are four ways to refer to record data:

- By [**provenance**](#provenance): the source that the record data originated from.
  The corresponding identifiers called *prov-id*s.
- By [**reference**](#references-to-provenance): a standardized way of referring to a specific provenance.
  The corresponding identifiers called *ref-id*s.
- By [**alias**](#aliases): an alternative name for provenance
  The corresponding identifiers called *alias*es.

### Provenance

A fundamental concept underlying how Autobib works is the notion of **provenance**: the *source that the record data originated from*.

All data stored by Autobib has provenance.
Most forms of provenance are external: for example, a *prov-id* `doi:1234/abcd` refers to the [DOI](https://www.doi.org/) `1234/abcd`.
There is one special type of provenance, called *local provenance*, which refers to data which you have manually input into your database.
This uses *prov-id*s of the form `local:xyz`.

Provenance is important since it can be used to retrieve data even when the data might not yet exist in your database.
If you make a request like `autobib get arxiv:2002.04575`, Autobib first checks if the data is present in your database.
If does not exist, the data will be retrieved first using the [arXiv API](https://info.arxiv.org/help/api/index.html), stored in your database, and then returned.

### References to provenance

Some identifiers of the form `provider:sub_id` can also be a **reference** to provenance.
The main example is data provided by [zbMATH](https://zbmath.org).
The internal zbMATH identifier is a 7 or 8 digit numeric code, like `01234567`, and is referred using the *prov-id* `zbmath:01234567`.
However, zbMATH also supports two other identifier types: JFM *ref-id*s, like `jfm:57.0055.01`, and Zbl *ref-id*s, like `zbl:0003.04901`.

These are also valid identifiers, but are internally converted directly to the provenance to which they refer.
One can think of references as a automatically assigned alternative names for provenance.

The current table is as follows:

- `arxiv`: provenance
- `doi`: provenance
- `isbn`: references `ol:`
- `jfm`: references `zbmath:`
- `local`: provenance
- `mr`: provenance
- `ol`: provenance
- `zbl`: references `zbmath:`
- `zbmath`: provenance

### Aliases

The standard way to refer to data in Autobib is by provenance or by reference.
This means using the key `doi:1234/abcd` directly in your in your files, etc.

However, since the provenance tends to be machine-readable instead of human-readable, Autobib also supports **aliases**.
These are keys of the form `xyz`, not containing a semicolon to distinguish from provenance, and not beginning with the reserved `#` character (for reasons that will become clear in the next section).
An alias is just *an alternative name for the provenance*.
If `xyz` is an alias for `doi:1234/abcd`, then writing `xyz` is equivalent to writing `doi:1234/abcd`.
The alias is also used as the citation key in the BibTeX output.

## Revisions

### The edit-tree

Instead of each record being unique, the Autobib database stores a tree containing all of the modifications associated with a *prov-id*.
When a record is modified, a new row is inserted containing the modified data along with a reference to the previous version.

At any point in time, there is a unique *active* record associated with the *prov-id*.
This is the record which is returned when, for example, you use `autobib get`.

There are three states that a record can be in.

1. `entry`: There is bibliographic data associated with the record.
2. `deleted`: This is a deletion marker indicating that there is no data.
   There may also be information about a replacement key.
   This state is created with `autobib delete` or `autobib delete --replace`.
3. `void`: This is a special state which is similar to there being no record present in the database.
   For example, this will cause `autobib get` to automatically retrieve new data for the row.
   It is uncommon for a record to be in this state, but this can be attained manually with `autobib hist void`, or is created automatically with `autobib hist rewind-all` or `autobib hist reset --before` when the threshold time precedes the existence of the record in the database.

Note that `void` entries still contain some state: it preserves aliases, and still tracks the *prov-id* which can be used to more efficiently look-up new data.

### Moving around the edit-tree

The simplest operations regarding revisions are `autobib hist undo` and `autobib hist redo`.
Undo sets the active state to the parent state, if the parent state exists.
Redo sets the active state to the newest child.
To disambiguate multiple children, it is also possible to pass an explicit index indicating which child branch to follow.

For more complex operations involving the edit-tree, the output of `autobib log --tree` can be useful.
This prints a branch diagram showing the relationship between all of the states in reverse chronological order.
The active state is also highlighted.

The branch diagram also includes special hexadecimal *revisions*, which can be used to refer to specific versions.
You can change to a specific revision using `autobib hist reset --rev`.

### Lifetimes

In most situations, the edit-tree consists of a number of distinct versions (obtained, say, with `autobib edit` or `autobib update`), and then potentially some deletion markers at node leaves.
Certain operations are not permitted by default.
For example, you cannot edit a deletion marker.

In common usage, you would not create new states beyond a deletion marker.
For example, attempting to edit or update a deletion marker will result in an error.
In particular, the default state of the edit-tree is that it forms a *lifetime*: a special tree where any non-leaf node contains entry data, and the leaf nodes may additionally be deleted.

However, it is possible to edit beyond a deleted state if desired: this is called *reviving* a deletion marker.
This is achieved either using `autobib revive` (which you must provide with new data) or with `autobib update --revive`, which inserts new data read from the data provider.

Operations involving multiple lifetimes often require special flags.

- Visualizing all lifetimes with `autobib log` requires the `--all` flag.
- `autobib hist undo` will not undo into a deleted state, unless you use `autobib hist undo --delete`
- `autobib hist redo` will not redo beyond a deleted state, unless you use `autobib hist redo --revive`.
