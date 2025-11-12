# Unreleased

Supported database versions: `<= 1`

Changes since `v0.4.1`.

## Breaking changes

- The `-f/--fields` option and the `--all-fields` flag in `autobib find` have been removed.
  Their functionality has been superseded by template support with the new `--template` option.
- Many options have been normalized and combined for better help messages and uniformity.
  1. The `--prefer-current` and `--prefer-incoming` flags have been removed from the `import`, `merge`, and `update` subcommands.
    They have been replaced with a `-n/--on-conflict` option which accepts an explicit argument which is the previous flag name.
    For example, `--prefer-current` is now `-n prefer-current` or `-nc` for short.
  2. The `--records` and `--attachments` flags have been removed from the `find` subcommand.
    They have been replaced with a `-m/--mode` option which accepts an explicit argument which is the previous flag name.
  3. The `--local`, `--determine-key`, `--retrieve`, and `--retrieve-only` flags have been removed from the `import`.
    They have been replaced with a `-m/--mode` option which accepts an explicit argument which is the previous flag name.
- The rules to determine default values have changed.
  Manual definition now always overrides defaults.
  - The `--no-interactive` flag is set by default if either STDIN or STDERR is non-interactive.
  - The `--on-conflict` option is set by default to `prefer-current` if either STDIN or STDERR is non-interactive, and to `prompt` otherwise.
  - Manual definitions can modify other defaults.
    The `--no-interactive` flag implies `--on-conflict prefer-current`, if `--on-conflict` is not manually set.

## New features

- `autobib attach` now accepts URLs as well as paths for the attachment.
- Added a `-t/--template` option for `autobib find` which allows manually specifying a template string to use when rendering.
  The precise expansion behaviour can also be modified with the `-s/--strict` flag.
  Read more in the [template syntax documentation](../template.md).
- `autobib source` now supports reading from standard input with the `--stdin` flag, which accepts a single argument specifying the file type of standard input.

## Changes to providers

- Fixed arXiv API parsing issues resulting from arXiv API format changes.
- Minor improvements to zbMATH response parsing.
