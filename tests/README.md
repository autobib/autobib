# Testing

The testing facade can benefit from local caching of response data to reduce the number of network requests required for the tests to succeed.
In order to generate the cache, run
```sh
xargs -- cargo run --locked --features write_response_cache -- -vv get --retrieve-only --ignore-null < 'tests/remotes.txt'
```
This will generate a file `responses.dat` in your working directory.
You can choose an alternative location by setting the `AUTOBIB_RESPONSE_CACHE_PATH` environment variable.

After generating the response cache, you can read from the response cache while testing by running
```sh
cargo test --features read_response_cache
```
This command can be run without network access.

## Automated testing

In order to automate the above steps, and also run additional checks and lints, you can use [`scripts/test.sh`](/scripts/test.sh):
```sh
./scripts/test.sh
```
The script respects the following variables:

- Set `LIBSQLITE3_SYS_USE_PKG_CONFIG=1` to run the tests using the SQLite library available on your system instead of a bundled copy of SQLite.
- Set `AUTOBIB_RUN_EXTENDED_CHECKS=0` to disable extended checks and lints which are only necessary during development.
- Set `AUTOBIB_RESPONSE_CACHE_DIR` to the directory to be used to store network request cache files.
  The default value is `$XDG_CACHE_HOME/autobib`.
  Individual network request cache files are stored using a hash scheme to invalidate incompatible cache files.
  This means that network cache files can be re-used for many test runs.

The script automatically generates the cache files in paths of the form `cache/test-cache-*/responses.dat`, and uses `cache/records.db` as a temporary database file to generate new caches.
You can safely delete the `cache` directory as its contents are automatically created if required.

With `AUTOBIB_RUN_EXTENDED_CHECKS=0` the script only depends on having a Rust toolchain.
Otherwise, the script has the following additional dependencies:

- [`shellcheck`](https://www.shellcheck.net/): for linting shell scripts
- [`deno`](https://deno.com/): for running markdown lints
