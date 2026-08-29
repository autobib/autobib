# Glossary

## Data terminology

**active record** &ensp; the unique record which is currently accessible in an edit-tree

**deleted record** &ensp; a special record that serves as a deletion marker

**edit-tree** &ensp; the tree of records associated with a canonical identifier containing the edit history of the identifier

**entry**, **entry record** &ensp; a record which contains bibliographic data

**entry data** &ensp; the bibliographic data associated with an entry

**record** &ensp; structured information in the local database associated with an identifier, often containing bibliographic data

**revision** &ensp; a record in the context of an edit-tree, uniquely identified by a hexadecimal string

**revision id** &ensp; a hexadecimal string that uniquely identifies a revision

**void record** &ensp; a special record for data which has been removed from the database

## Identifier terminology

**alias** &ensp; a custom key which is used in place of an identifier and does not contain a colon `:`

**canonical identifier** &ensp; an identifier in direct association with an active record, in contrast to a reference identifier

**identifier** &ensp; a key of the form `provider:sub_id`

**key** &ensp; a text string referring to an active record

**local identifier** &ensp; a canonical identifier for data without a named provider

**provenance** &ensp; the source that the data in a record originates from

**provider** &ensp; a source from which record data can be obtained; the part before the `:` in an identifier.

**reference identifier** &ensp; an identifier which refers to a canonical identifier

**remote identifier** &ensp; a canonical or reference identifier referring to data which originates from a named provider

**sub-id** &ensp; a special text string used by a provider to identify bibliographic data; the part after the `:` in an identifier
