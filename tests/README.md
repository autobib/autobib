# Testing

The testing facade can benefit from local caching of response data to reduce the number of network requests required for the tests to succeed.
In order to generate the cache, run
```sh
xargs -a 'tests/remotes.txt' -d '\n' -- cargo run --features write_response_cache -- -vv get --retrieve-only --ignore-null
```
This will generate a file `responses.dat` in your working directory.
You can choose an alternative location by setting the `AUTOBIB_RESPONSE_CACHE_PATH` environment variable.

After generating the response cache, you can (optionally) read from the response cache while testing by running
```sh
cargo test --features read_response_cache
```

## Automated testing
In order to automate the above steps, and also run other checks, you can use [`scripts/test.sh`](../scripts/test.sh):
```sh
./scripts/test.sh
```
This script has the following dependencies:
- [`shellcheck`](https://www.shellcheck.net/)

The script automatically generates the cache files in paths the form `cache/test-cache-*/responses.dat`, and uses `cache/records.db` as a temporary database file to generate new caches.
You can safely delete the `cache` directory as its contents are automatically created if required.
