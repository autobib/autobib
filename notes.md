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
- By [**revision**](#working-with-revision-history): a reference to a specific data version
  The corresponding identifiers called *revision*s.

## Provenance

A fundamental concept underlying how Autobib works is the notion of **provenance**: the *source that the record data originated from*.

All data stored by Autobib has provenance.
Most forms of provenance are external: for example, a *prov-id* `doi:1234/abcd` refers to the [DOI](https://www.doi.org/) `1234/abcd`.
There is one special type of provenance, called *local provenance*, which refers to data which you have manually input into your database.
This uses *prov-id*s of the form `local:xyz`.

Provenance is important since it can be used to retrieve data even when the data might not yet exist in your database.
If you make a request like `autobib get arxiv:2002.04575`, Autobib first checks if the data is present in your database.
If does not exist, the data will be retrieved first using the [arXiv API](https://info.arxiv.org/help/api/index.html), stored in your database, and then returned.

## References to provenance

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

## Aliases

The standard way to refer to data in Autobib is by provenance or by reference.
This means using the key `doi:1234/abcd` directly in your in your files, etc.

However, since the provenance tends to be machine-readable instead of human-readable, Autobib also supports **aliases**.
These are keys of the form `xyz`, not containing a semicolon to distinguish from provenance, and not beginning with the reserved `#` character (for reasons that will become clear in the next section).
An alias is just *an alternative name for the provenance*.
If `xyz` is an alias for `doi:1234/abcd`, then writing `xyz` is equivalent to writing `doi:1234/abcd`.
The only difference is the name used, including as the citation key in the BibTeX output.

## Working with revision history

Internally, Autobib keeps a full revision history associated with each provenance.
Whenever you make a change to a record, or update it, or soft delete it, the older data remains inside the database and instead a new revision is added and becomes the 'active' revision.

However, it is often important to refer to a specific revision.
For example, `autobib reset` changes the current active version to a specific revision.
It may also be desirable to obtain data associated with a specific revision, regardless of the most up-to-date version of a given character.

A **revision** is a `#` character, followed by a hexadecimal string, for example `#a1b23`.
A revision may also be preceded with trailing zeros or use capital letters, so the above example is equivalent to `#00A1b23`.

> [!CAUTION]
> Revisions are *non-portable* and *unstable*.
> Revisions which are already present in the database are guaranteed to not change, but deleting a specific revision may result in that revision being reused by new data inserted into the database.
