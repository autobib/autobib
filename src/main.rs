mod app;
pub mod cite_search;
mod config;
pub mod db;
mod entry;
pub mod error;
pub mod format;
mod http;
mod logger;
mod normalize;
mod output;
mod path_hash;
pub mod provider;
mod record;
pub mod term;

use std::{io, process::exit};

use clap::{CommandFactory, Parser};
use clap_complete::aot::generate;

use self::{
    app::{Cli, Command, run_cli},
    db::CitationKey,
    entry::RawRecordData,
    logger::{Logger, reraise},
};

pub use self::{
    config::Config,
    entry::Entry,
    normalize::{Normalization, Normalize},
    record::{Alias, AliasOrRemoteId, MappedKey, RecordId, RemoteId, get_record_row},
    term::{Confirm, Editor, EditorConfig},
};

static LOGGER: Logger = Logger {};

fn main() {
    #[cfg(not(debug_assertions))]
    std::panic::set_hook(Box::new(|panic_info| {
        eprintln!(
            "An unexpected error occured while running the program:
  1. Your database file could be malformed or has been edited by another program.
     Run `autobib util check` to see if this is the case.
  2. If you have ruled out 1., this is a bug in autobib. Please report it at
     > https://github.com/autobib/autobib/issues
     including the error message below and any other information you can provide
     about the context in which it occured.

The following is a description of the error which occured:
"
        );
        eprintln!("{panic_info}");
    }));

    let cli = Cli::parse();

    // generate completions upon request and exit
    if let Command::Completions { shell } = cli.command {
        let mut clap_command = Cli::command();
        let bin_name = clap_command.get_name().to_owned();
        generate(shell, &mut clap_command, bin_name, &mut io::stdout());
        return;
    }

    // perform custom validation / normalization
    cli.validate();

    // initialize logger
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(cli.verbose.log_level_filter()))
        .unwrap();

    #[cfg(not(any(feature = "write_response_cache", feature = "read_response_cache")))]
    let client = http::UreqClient::new();

    #[cfg(all(feature = "write_response_cache", not(feature = "read_response_cache")))]
    let client = http::cache::LocalWriteClient::new();

    #[cfg(feature = "read_response_cache")]
    let client = http::cache::LocalReadClient::new();

    // run the cli
    if let Err(err) = run_cli(cli, &client) {
        reraise(&err);
    }

    // check if there was a non-fatal error during execution
    if Logger::has_error() {
        exit(1)
    }

    #[cfg(all(feature = "write_response_cache", not(feature = "read_response_cache")))]
    client.serialize()
}
