# Unreleased

This version migrates the database version from `2` to `3`.
To run the migration code, report the database version, and validate your local files after updating, run
```sh
autobib -v util check
```

Supported database versions: `<= 3`

Changes since `v0.6.1`.

## Breaking changes

- `zbmath` identifiers are now stored internally without 0-padding to length 8
- SQLite is now only bundled when the Cargo feature `bundled-sqlite` is enabled.
  This feature is enabled by default, but this may cause breakage with builds using `--no-default-features`.
  Disabling this feature will cause the compiled binary to link to your SQLite system library instead.
- In a future version, Autobib will migrate to a new attachment folder format.
  The new folder format is not compatibile with autobib versions `< 0.7.0`.
  - This version is able to read both the legacy format and the new format.
  - You can migrate early by running `autobib util migrate-attachments`.
    Note that the attachments will no longer be readable by an old version of Autobib.
- Renamed `autobib util list` to `autobib util print-identifiers`.
  `autobib util list` is still usable as an alias, but this will be removed in the future.
- Aliases can no longer contain control characters.
  - Existing aliases containing control characters can still be accessed and renamed.
- `autobib find --mode canonical-id` has been renamed to `autobib find --mode records`.
  The CLI still accepts the old name as an alias, but this will be removed in the future.
- `autobib get` has been reworked.
  Output is no longer sorted and deduplicated: instead, one record is printed for each identifier.
  This also means that the `--out` and `--append` options have been removed.
  This behaviour is now included in `autobib source`.
  There are also new features:
  - Added `--template` option, to format records with an arbitrary template instead of as BibTeX.
  - Added the ability to read from stdin.
- `autobib util list` has been renamed to `autobib list`.
  - Added optional glob pattern argument to filter identifiers.
  - Added `--template` flag to format output instead of printing identifiers.

## New features

- New template metas:
  - `%json`, a JSON dictionary of all of the available data
  - `%modified`, the modification time of the record
- Added basic support for sourcing from [Typst](https://typst.app) files (using `autobib source file.typ`)
- Added support for multiple selections in `autobib find` by default.
  - The number of selections can be limited with the `--limit` option.
  - Single-selection mode can be re-enabled with `-1/--one`
- Added `autobib info -r preferred` to print the preferred identifier associated with a record.
- Added `autobib source --json`, which outputs a JSON dictionary mapping citation keys to record data.
