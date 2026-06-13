# Unreleased

Changes since `v0.6.1`.

## Breaking changes

- SQLite is now only bundled when the Cargo feature `bundled-sqlite` is enabled.
  This feature is enabled by default, but this may cause breakage with builds using `--no-default-features`.
  Disabling this feature will cause the compiled binary to link to your SQLite system library instead.
- Renamed `autobib util list` to `autobib util print-identifiers`.
  `autobib util list` is still usable as an alias, but this will be removed in the future.

## New features

- New command `autobib list` which prints records using the template syntax.
- Added new template meta `%bibtex`, which expands to the full BibTeX record.
- Adds basic support for sourcing from [Typst](https://typst.app) files (using `autobib source file.typ`)
