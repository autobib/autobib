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

## New features

- New command `autobib format` which prints records using the template syntax.
- New template metas:
  - `%bibtex`, which expands to the full BibTeX record.
  - `%json`, which JSON encodes the record data (excluding the canonical identifier)
- Added basic support for sourcing from [Typst](https://typst.app) files (using `autobib source file.typ`)
- Added support for multiple selections in `autobib find` by default.
  - The number of selections can be limited with the `--limit` option.
  - Single-selection mode can be re-enabled with `-1/--one`
- Added `autobib info -r preferred` to print the preferred identifier associated with a record.
