# Unreleased

This version migrates the database version from `2` to `3`.
To run the migration code, report the database version, and validate your local files after updating, run
```sh
autobib -v util check
```

Supported database versions: `<= 3`

Changes since `v0.5.1`.

## Breaking changes

- `zbmath` identifiers are now stored internally without 0-padding to length 8

## Fixes

- `zbmath` provider fixes (thanks @tornaria for the reports!):
  - Capture `year` field in more cases
  - Better handling of unknown link types
  - Handle identifiers of any length, instead of only allowing length `7` or `8`
