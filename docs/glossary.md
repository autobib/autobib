# Glossary

## Data terminology

**active record** &ensp; the unique record in the edit-tree which is currently accessible

**deleted record** &ensp; a record which is a deletion marker

**edit-tree** &ensp; the tree of records associated with a canonical identifier containing the history

**entry**, **entry record** &ensp; a record which contains bibliographic data

**entry data** &ensp; the bibliographic data associated with an entry

**record** &ensp; structured information in the local database associated with an identifier

**revision** &ensp; a hexadecimal string in one-to-one correspondence with records

**void record** &ensp; a special record for data which has been removed from the database

## Identifier terminology

**alias** &ensp; a custom key referring to an identifier which does not containing a colon `:`

**canonical identifier** &ensp; an identifier uniquely associated with the active record

**identifier** &ensp; a key of the form `provider:sub_id`

**key** &ensp; a text string referring to an active record

**local identifier** &ensp; a canonical identifier for data without a named provider

**provenance** &ensp; the specific origin of record data

**provider** &ensp; a named source from which record data can be obtained; the part before the `:` in an identifier.

**reference identifier** &ensp; an additional identifier which refers to a canonical identifier

**remote identifier** &ensp; a canonical or reference identifier referring to data which originates from a named provider

**sub-id** &ensp; a special identifier used by a provider; the part after the `:` in an identifier
