# Unreleased

Supported database versions: `<= 1`

Changes since `v0.5.0`.

## Other changes

- When prompting for user input, the prompt text is now printed to stderr instead of stdout.

## Fixes

- Fixed a bug where the program panicked on broken pipe errors when writing to stdout.
  Now the program terminates silently on such errors.
- Fixes an unexpected error when passing a template string containing an invalid JSON literal.
