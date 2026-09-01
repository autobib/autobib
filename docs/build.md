# Build documentation

Jump to:

- [Pre-compiled release binaries](#pre-compiled-release-binaries)
- [Installing from source](#installing-from-source)
- [Running tests locally](#running-tests-locally)
- [Build configuration](#build-configuration)

## Pre-compiled release binaries

For convenience, pre-compiled binaries are available on the [GitHub releases page](https://github.com/autobib/autobib/releases).
Starting in Autobib 0.8.0, GitHub releases are [immutable](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases).
In short, this means that:

- Version tags cannot be moved.
- Release assets cannot be changed.

GitHub has [documentation for verifying releases](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/secure-your-dependencies/verify-release-integrity).

### Release asset format (`>= 0.8.0`)

Release assets are organized by platform target and named using the convention `autobib-$VERSION-$TARGET.$SUFFIX`.
They can be downloaded at the URL
```text
https://github.com/autobib/autobib/releases/download/v$VERSION/autobib-$VERSION-$TARGET.$SUFFIX
```
The variables have the following meaning:

- `$VERSION`: The version, such as 0.8.0.
- `$TARGET`: The [Rust platform target](https://doc.rust-lang.org/rustc/platform-support.html), such as `aarch64-apple-darwin`.
  You can view your current platform target with `rustc --print host-tuple`.
- `$SUFFIX`: The compression suffix.
  For Windows assets, this is `zip`; otherwise this is `tar.gz`.

In this example, the asset filename and URL are
```text
autobib-0.8.0-aarch64-apple-darwin.tar.gz
https://github.com/autobib/autobib/releases/download/v0.8.0/autobib-0.8.0-aarch64-apple-darwin.tar.gz
```

SHA-256 checksums for all release archives can be found in the `SHA256SUMS` file.
Note that SHA-256 checksums for individual assets are also automatically computed by GitHub and can be viewed on the releases page or programmatically using the GitHub API.

Each compressed archive contains the compiled binaries and also bundles source files and generated files:

- The compiled binaries `$BIN` (or `$BIN.exe` on Windows).
  Currently this is only `autobib`.
- `CHANGELOG.md`: The changelog.
- `COPYRIGHT`: A copyright notice.
- `LICENSE`: A copy of the licence.
- `README.md`: A copy of the README.
- `third-party-licenses.html`: A copy of all of the licences of all dependencies included in the build.

Visit the release page to obtain a full list of available release assets.
You can query for available assets programmatically using the GitHub API or by reading the `SHA256SUMS` file.
Note that availability of pre-built release assets is subject to change, depending on free GitHub CI allowances for public repositories.

### Release asset format (`< 0.8.0`)

The asset format is not specified; please visit the individual release pages.

## Installing from source

To install from source, an up-to-date stable Rust toolchain is required.

1. The easiest way to install from source is to use the published version on [crates.io](https://crates.io).
  In short, for a published version X.Y.Z, run

    ```sh
    cargo install --locked autobib@X.Y.Z
    ```

2. You can also download a source tarball (or ZIP file) directly from GitHub:

    ```sh
    VERSION=X.Y.Z
    curl -fL "https://github.com/autobib/autobib/archive/refs/tags/v$VERSION.tar.gz" | tar -xz
    cargo install --locked --path "autobib-$VERSION"
    ```

3. Finally, if you would like to an unstable development build from `main`, you can get it using Git:

    ```sh
    git clone --depth 1 https://github.com/autobib/autobib
    cargo install --locked --path autobib
    ```

In any of the three cases, use `cargo install --root <PATH>` to specify a custom installation folder, in which case Autobib will be installed at `<PATH>/bin/autobib` (or `<PATH>/bin/autobib.exe` on Windows).
Note that the GitHub tarball format is subject to change so it is not recommended to store or use a SHA-256 hash for validation.

For immutable releases, release tags cannot change, so the tag alone uniquely specifies a release version.
GitHub has [documentation for verifying releases](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/secure-your-dependencies/verify-release-integrity).
You can verify immutability using the GitHub CLI:
```sh
gh --repo autobib/autobib release verify vX.Y.Z
```

## Running tests locally

Autobib is currently tested in CI on the following platforms:

- Ubuntu x86\_64 and ARM64
- macOS ARM64
- Windows x86\_64 and ARM64

Note that the tested platforms are subject to change, depending on free GitHub CI allowances for public repositories and Rust platform support.

In particular, you may wish to run tests for your own platform.
Note that directly running `cargo test` will result in a large number of parallel network requests, which can result in spurious test failures and rate-limited test runners.
We therefore provide a [test script](/scripts/test.sh) which first builds a network facade and then uses that facade to execute tests.
In short, it is recommended to use
```sh
AUTOBIB_RUN_EXTENDED_CHECKS=0 scripts/test.sh
```
Read the [test docs](/tests/README.md) for more detail.

## Build configuration

### Bundling SQLite

By default, Autobib compiles with a bundled copy of SQLite enabled by the `bundled-sqlite` Cargo feature.
To link against the SQLite library available on your system instead, disable this feature:

```sh
cargo install --locked autobib --no-default-features
```
This makes the binary about 1.5MB smaller and reduces compile time, at the cost of potential compatibility issues and a few additional runtime checks.
Note that the system SQLite library must be version 3.35.0 or newer.
If this is not the case, Autobib will fail with a runtime error.

Note that Autobib is only tested against the bundled SQLite configuration by default, so using the system SQLite copy may result in difficult-to-diagnose errors.
You can run the test script to check compatibility with your system SQLite library using
```sh
AUTOBIB_RUN_EXTENDED_CHECKS=0 LIBSQLITE3_SYS_USE_PKG_CONFIG=1 scripts/test.sh
```
Note that the bundled copy of SQLite is compiled with the following flags:

- `SQLITE_DEFAULT_MEMSTATUS=0`
- `SQLITE_DEFAULT_WAL_SYNCHRONOUS=1`
- `SQLITE_DQS=0`
- `SQLITE_LIKE_DOESNT_MATCH_BLOBS`
- `SQLITE_MAX_EXPR_DEPTH=0`
- `SQLITE_OMIT_DEPRECATED`
- `SQLITE_OMIT_PROGRESS_CALLBACK`
- `SQLITE_OMIT_SHARED_CACHE`
- `SQLITE_STRICT_SUBTYPE=1`

### Dynamically link musl targets

By default, musl targets are statically linked.
In order to dynamically link on musl, the simplest way is to manually overwrite `RUSTFLAGS`.
For example:
```sh
RUSTFLAGS="-C target-feature=-crt-static" cargo build --locked --release --target x86_64-unknown-linux-musl
```
You can also manually override the target configuration using the `--config` option of `cargo build`.
