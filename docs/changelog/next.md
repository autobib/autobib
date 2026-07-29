# Unreleased

This version migrates the database version from `2` to `4`.
To run the migration code, report the database version, and validate your local files after updating, run
```sh
autobib -v util check
```

Supported database versions: `<= 4`

Changes since `v0.6.1`.

## Breaking changes

- `zbmath` identifiers are now stored internally without 0-padding to length 8
- SQLite is now only bundled when the Cargo feature `bundled-sqlite` is enabled.
  This feature is enabled by default, but this may cause breakage with builds using `--no-default-features`.
  Disabling this feature will cause the compiled binary to link to your SQLite system library instead.
- `autobib find --mode canonical-id` has been renamed to `autobib find --mode records`.
  The CLI still accepts the old name as an alias, but this will be removed in the future.
- `autobib get` has been reworked.
  Output is no longer sorted and deduplicated: instead, one record is printed for each identifier.
  This also means that the `--out` and `--append` options have been removed, and warnings are no longer printed for duplicate records.
  The previous behaviour can be reproduced using `autobib source`.
- `autobib util list` has been renamed to `autobib list`.
  - Added optional glob pattern argument to filter identifiers.
  - Added `--template` flag to format output instead of printing identifiers.
- The configuration value `preferred_providers` has been replaced by `preferred_keys`.
  - `preferred_keys` is more general and can contain a list of regexes to match keys
  - Migrate `preferred_providers` to `preferred_keys` by replacing each `provider` with the corresponding regex `^provider:.*`
- `autobib info` format with `--report all` has changed.
  Note that `autobib info` is not intended to be machine-readable; use `autobib info --json` instead.
- Aliases can no longer contain control characters (such as tab `\t` or newline `\n`).
  - Existing aliases containing control characters can still be accessed and renamed.
- Entry types `comment`, `preamble`, and `string` are now disallowed since these names are reserved by BibTeX.

## Deprecations and future breaking changes

- In a future version, Autobib will migrate to a new attachment folder format.
  The new folder format is not compatible with Autobib versions `< 0.7.0`.
  - This version is able to read both the legacy format and the new format.
  - You can migrate early by running `autobib clean attachments --migrate`.
    Note that attachments will no longer be readable by old version of Autobib.

## New features

- New template metas:
  - `%json`, a JSON dictionary of all of the available data
  - `%modified`, the modification time of the record
  - `%key`, the original citation key from the request
- Added basic support for sourcing from [Typst](https://typst.app) files (using `autobib source file.typ`)
- Added support for multiple selections in `autobib find` by default.
  - The number of selections can be limited with the `--limit` option.
  - Single-selection mode can be re-enabled with `-1/--one`
- Added `autobib info -r preferred` to print the preferred identifier associated with a record.
- Added `autobib source --json`, which outputs a JSON dictionary mapping citation keys to record data.
- Added `autobib info --json` to format the report as appropriate JSON.
- Added `autobib backup` to backup the record SQL database to a separate file.
- Added `autobib get --template`, to format records with an arbitrary template instead of as BibTeX.

## Changes

- Improved handling of attachment directories with `autobib replace` and `autobib delete`:
  - `replace` relocates attachment directories to the new location
  - `delete` warns on orphaned attachment directories
  - This behaviour can be disabled with the `-A` or `--ignore-attachments` option
- `autobib get` now also reads keys from standard input, one per line (with whitespace stripped)

## Fixes

- Adjust to new zbMath API format for null records.
