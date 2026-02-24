# Glossary of terminology

## Data terminology

- *record*: a snapshot of bibliographic data along with associated metadata
- *record data*: the bibliographic data associated with a record
- *revision*: a hexadecimal string in one-to-one correspondence with records
- *edit-tree*: the tree of records associated with a canonical identifier containing the history
- *active record*: the unique record in the edit-tree which is currently accessible by an identifier

## Identifier terminology

- *identifier*: a text string referring to an active record
- *canonical identifier*: a special identifier uniquely associated with the active record
- *reference identifier*: additional identifiers which refer to a canonical identifier
- *alias*: a custom identifier not containing a colon `:`
- *provenance*: the specific origin of record data
- *provider*: a named source from which record data can be obtained; the part before the `:` in a canonical or reference identifier.
- *sub-id*: a special identifier used by a provider; the part after the `:` in a canonical or reference identifier
- *local identifier*: a special canonical identifier for data without a named provider
- *remote identifier*: a canonical or reference identifier referring to data which originates from a named provider
