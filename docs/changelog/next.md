# Unreleased

Supported database versions: `<= 2`

Changes since `v0.5.1`.

## Breaking changes

- `autobib delete` now performs 'soft-deletion' by default, which does not remove the record from your database but instead inserts a deletion marker.
  The old behaviour can be obtained with `autobib delete --hard`.
  It is no longer necessary to pass all identifiers when performing a deletion.
  Passing redundant identifiers will now result in an error.
- `autobib merge` has been removed.
  It is no longer possible to merge records together.
  This functionality has been replaced by `autobib delete --replace <replacement>`, which performs a soft delete and stores a replacement identifier.
  Subsequent requests for the deleted identifier will return an error message suggesting the replacement.
  Also see `autobib update --from-record` and `autobib delete --update-aliases`.
- `autobib edit` no longer opens an interactive editor if headless edit methods are specified.
- `autobib update --from` has been renamed to `autobib update --from-bibtex`.
- `autobib update` can no longer be used to retrieve new data for null records.
  To retrieve data, first delete the null record using `autobib util evict`.
- `autobib local` no longer creates an alias automatically.
- `autobib import` has been re-implemented.
  Run `autobib help import` for more detail.
  Most notably:
  - Import modes no longer exist.
    The default behaviour is to attempt to determine a canonical identifier, and will result in an error if no identifier can be found.
  - Importing now automatically skips existing data present in your database.
    To insert updated data, use `autobib import --update`.
  - Retrieving data when importing is no longer possible, but reference identifiers can be mapped to canonical identifiers using `--resolve`.

## New features

- Autobib now has robust support for edit history.
  This includes sub-commands such as undo, redo, soft-deletion, reset, and time-travel, as well as sub-commands for manipulating the edit history.
  See the [data model documentation](/docs/data_model.md) and `autobib help hist` for more detail.
- A command `autobib log` has been added to show the edit history associated with an identifier.
- `autobib edit` now supports headless edit methods.
  Run `autobib help edit` for more detail.
- `autobib update` now supports updating from data present in other records in your database with `autobib update --from-record`.
- `autobib local` now supports headless methods to creating the local record from data specified at the command line.
- `autobib import --include-files` imports files specified in the `file = {...}` field of entries in the imported bibliography.
- A command `autobib dedup` has been added, which attempts to find replacement records which are equivalent to a given record.
  Currently, it is only possible to deduplicate based on identifiers present in the record data.

## Fixes

- `autobib update` now normalizes incoming data using the `on_insert` rules in the configuration.
- Substantial performance improvements in some cases when working with very large databases with proper use of SQL indices.
