# Glossary of terminology

## Data terminology

- *record*: structured information in the local database associated with an identifier
- *entry*, *entry record*: a record which contains bibliographic data
- *entry data*: the bibliographic data associated with an entry
- *deleted record*: a record which is a deletion marker
- *void record*: a special record for data which has been removed from the database
- *revision*: a hexadecimal string in one-to-one correspondence with records
- *edit-tree*: the tree of records associated with a canonical identifier containing the history
- *active record*: the unique record in the edit-tree which is currently accessible

## Identifier terminology

- *key*: a text string referring to an active record
- *identifier*: a key of the form `provider:sub_id`
- *canonical identifier*: an identifier uniquely associated with the active record
- *reference identifier*: an additional identifier which refers to a canonical identifier
- *alias*: a custom key referring to an identifier which does not containing a colon `:`
- *provenance*: the specific origin of record data
- *provider*: a named source from which record data can be obtained; the part before the `:` in an identifier.
- *sub-id*: a special identifier used by a provider; the part after the `:` in an identifier
- *local identifier*: a canonical identifier for data without a named provider
- *remote identifier*: a canonical or reference identifier referring to data which originates from a named provider
