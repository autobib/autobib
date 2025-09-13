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
