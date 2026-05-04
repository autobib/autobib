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

## Changes

- Autobib is migrating to a new attachment folder format.
  The new folder format is not compatibile with autobib versions `< v0.7.0`.
- This version is able to read both the legacy format and the new format.
- Adds a command `autobib util migrate-attachments` which migrates the attachment format to the new format.
