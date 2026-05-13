fn main() {
    println!("cargo::rerun-if-env-changed=LIBSQLITE3_SYS_USE_PKG_CONFIG");

    let bundled_sqlite = std::env::var_os("CARGO_FEATURE_BUNDLED_SQLITE").is_some();
    let use_pkg_config =
        std::env::var_os("LIBSQLITE3_SYS_USE_PKG_CONFIG").map(|value| value != "0");

    match (bundled_sqlite, use_pkg_config) {
        (true, Some(true)) => {
            println!(
                "cargo::error=`LIBSQLITE3_SYS_USE_PKG_CONFIG` requests system SQLite via \
                 pkg-config, but the `bundled-sqlite` Cargo feature is enabled. Set \
                 `LIBSQLITE3_SYS_USE_PKG_CONFIG=0` or disable the `bundled-sqlite` feature."
            );
            std::process::exit(1);
        }
        (false, Some(false)) => {
            println!(
                "cargo::error=`LIBSQLITE3_SYS_USE_PKG_CONFIG=0` disables system SQLite via \
                 pkg-config, but the `bundled-sqlite` Cargo feature is disabled. Set \
                 `LIBSQLITE3_SYS_USE_PKG_CONFIG=1` or enable the `bundled-sqlite` feature."
            );
            std::process::exit(1);
        }
        _ => {}
    }
}
