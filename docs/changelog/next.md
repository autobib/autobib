# Unreleased

Changes since `v0.6.1`.

## Breaking changes

- SQLite is now only bundled when the Cargo feature `bundled-sqlite` is enabled.
  This feature is enabled by default, but this may cause breakage with builds using `--no-default-features`.
  Disabling this feature will cause the compiled binary to link to your SQLite system library instead.
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

- New command `autobib format` which prints records using the template syntax.
- New template metas:
  - `%json`, a JSON dictionary of all of the available data
  - `%modified`, the modification time of the record
- Added basic support for sourcing from [Typst](https://typst.app) files (using `autobib source file.typ`)
- Added support for multiple selections in `autobib find` by default.
  - The number of selections can be limited with the `--limit` option.
  - Single-selection mode can be re-enabled with `-1/--one`
- Added `autobib info -r preferred` to print the preferred identifier associated with a record.
- Added `autobib source --json`, which outputs a JSON dictionary mapping citation keys to record data.
