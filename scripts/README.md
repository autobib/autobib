# Convenience scripts

This directory contains `bash` scripts to automate certain functionality.

- [`test.sh`](test.sh): Automate tests with a local cache mechanism to reduce network requests. Depends on:
  - [`shellcheck`](https://www.shellcheck.net/)
- [`release.sh`](release.sh): Automate some release steps. Depends on:
  - [`sed`](https://www.gnu.org/software/sed/) (GNU-compatible)
  - [`cargo-edit`](https://crates.io/crates/cargo-edit)
  - [`deno`](https://https://deno.com/)
- [`create_test_db.sh`](create_test_db.sh): Create a test database with some undo history.
