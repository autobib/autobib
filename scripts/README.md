# Convenience scripts
This directory contains `bash` scripts to automate certain functionality.

- [`test.sh`](test.sh): Automate tests with a local cache mechanism to reduce network requests. Depends on:
  - [`shellcheck`](https://www.shellcheck.net/)
- [`release.sh`](release.sh): Automate some release steps. Depends on:
  - [`sed`](https://www.gnu.org/software/sed/) (GNU-compatible)
  - [`cargo-edit`](https://crates.io/crates/cargo-edit)
  - [`yq`](https://mikefarah.gitbook.io/yq)
