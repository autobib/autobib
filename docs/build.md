# Build configuration

## Bundling SQLite

By default, Autobib compiles with a bundled copy of SQLite enabled by the `bundled-sqlite` Cargo feature.
To link against the SQLite library available on your system instead, disable this feature:

```sh
cargo install --locked autobib --no-default-features
```
This makes the binary about 1.5MB smaller, at the cost of potential compatibility issues.
Note that the system SQLite library must be version 3.35.0 or newer.
If this is not the case, Autobib will fail with a runtime error.

Note that Autobib is only tested against the bundled SQLite configuration by default, so using the system SQLite copy may result in difficult-to-diagnose errors.
You can run the test script to check compatibility with your system SQLite library using
```sh
LIBSQLITE3_SYS_USE_PKG_CONFIG=1 ./scripts/test.sh
```
Note that the bundled copy of SQLite is compiled with the following flags:
```sh
SQLITE_DEFAULT_MEMSTATUS=0
SQLITE_DEFAULT_WAL_SYNCHRONOUS=1
SQLITE_DQS=0
SQLITE_LIKE_DOESNT_MATCH_BLOBS
SQLITE_MAX_EXPR_DEPTH=0
SQLITE_OMIT_DEPRECATED
SQLITE_OMIT_PROGRESS_CALLBACK
SQLITE_OMIT_SHARED_CACHE
SQLITE_STRICT_SUBTYPE=1
```

## Dynamically link musl targets

By default, musl targets are statically linked.
In order to dynamically link on musl, the simplest way is to manually overwrite `RUSTFLAGS`:
```sh
RUSTFLAGS="" cargo build --release
```
You can also manually override the target configuration using the `--config` option of `cargo build`.
