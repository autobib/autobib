mod app;
pub mod cite_search;
mod config;
pub mod db;
mod entry;
pub mod error;
mod http;
mod logger;
mod normalize;
mod path_hash;
pub mod provider;
mod record;
pub mod term;

use std::{
    io::{self, IsTerminal},
    process::exit,
};

use clap::{CommandFactory, Parser};
use clap_complete::aot::generate;

use self::{
    app::{run_cli, Cli, Command},
    db::CitationKey,
    entry::RawRecordData,
    logger::{error, info, Logger},
};

pub use self::{
    config::Config,
    entry::Entry,
    http::HttpClient,
    normalize::{Normalization, Normalize},
    record::{get_record_row, Alias, AliasOrRemoteId, MappedKey, RecordId, RemoteId},
    term::{Confirm, Editor, EditorConfig},
};

static LOGGER: Logger = Logger {};

fn main() {
    let mut cli = Cli::parse();

    // initialize logger
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(cli.verbose.log_level_filter()))
        .unwrap();

    // generate completions upon request and exit
    if let Command::Completions { shell } = cli.command {
        let mut clap_command = Cli::command();
        let bin_name = clap_command.get_name().to_owned();
        generate(shell, &mut clap_command, bin_name, &mut io::stdout());
        return;
    }

    // Check if stdin and stderr are terminals; if not, set no_interactive to 'false'
    if !(cli.no_interactive || io::stdin().is_terminal() && io::stderr().is_terminal()) {
        info!("Detected non-interactive input; auto-enabling `--no-interactive`.");
        cli.no_interactive = true;
    }

    // run the cli
    if let Err(err) = run_cli(cli) {
        error!("{err}");
    }

    // check if there was a non-fatal error during execution
    if Logger::has_error() {
        exit(1)
    }
}
