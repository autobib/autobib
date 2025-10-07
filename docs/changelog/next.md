# Unreleased

Supported database versions: `<= 1`

Changes since `v0.4.1`.

## Breaking changes

- The `-f/--fields` and `--all-fields` flags in `autobib find` have been removed.
  Their functionality has been superseded by template support with the new `--template` flag.

## New features

- `autobib attach` now accepts URLs as well as paths for the attachment.
- Added a `-t/--template` flag for `autobib find` which allows manually specifying a template string to use when rendering.
  The precise expansion behaviour can also be modified with the `-s/--strict` flag.
  Read more in the [template syntax documentation](../template.md).
