mod citekey;
pub mod database;
mod entry;
pub mod error;
mod http;
mod record;
pub mod source;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use clap_verbosity_flag::{Verbosity, WarnLevel};
use itertools::Itertools;
use log::{error, warn};
use xdg::BaseDirectories;

use citekey::{get_citekeys, guess_source_file_type, SourceFileType};
pub use database::{CitationKey, RecordDatabase};
use entry::KeyedEntry;
pub use http::HttpClient;
pub use record::{get_record, Alias, RecordId, RemoteId};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(long)]
    database: Option<PathBuf>,

    #[command(flatten)]
    verbose: Verbosity<WarnLevel>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage aliases.
    #[command(alias = "a")]
    Alias {
        #[command(subcommand)]
        alias_command: AliasCommand,
    },
    /// Retrieve records given citation keys.
    #[command(alias = "g")]
    Get {
        /// The citation keys to retrieve.
        citation_keys: Vec<String>,
    },
    /// Show metadata for citation key.
    #[command()]
    Show,
    /// Generate records by searching for citation keys inside files.
    #[command(alias = "s")]
    Source {
        /// The files in which to search.
        paths: Vec<PathBuf>,
        /// Override file type detection.
        #[arg(long)]
        file_type: Option<SourceFileType>,
    },
}

#[derive(Subcommand)]
enum AliasCommand {
    Add { alias: Alias, target: RecordId },
    Delete { alias: Alias },
    Rename { alias: Alias, new: Alias },
}

fn main() {
    let cli = Cli::parse();

    // initialize warnings
    if let Some(level) = cli.verbose.log_level() {
        stderrlog::new()
            .module(module_path!())
            .verbosity(level)
            .init()
            .unwrap();
    }

    // run the cli
    if let Err(err) = run_cli(cli) {
        error!("{err}")
    }
}

fn run_cli(cli: Cli) -> Result<()> {
    // Open or create the database
    let mut record_db = if let Some(db_path) = cli.database {
        // at a user-provided path
        RecordDatabase::open(db_path)?
    } else {
        // at the default path
        let xdg_dirs = BaseDirectories::with_prefix("autobib")?;
        RecordDatabase::open(xdg_dirs.place_data_file("cache.db")?)?
    };

    // Initialize the reqwest Client
    let client = HttpClient::new()?;

    match cli.command {
        Command::Alias { alias_command } => match alias_command {
            AliasCommand::Add { alias, target } => {
                // first retrieve 'target', in case it does not yet exist in the database
                get_record(&mut record_db, target.clone(), &client)?;
                // then link to it
                record_db.insert_alias(&alias, &target)?;
            }
            AliasCommand::Delete { alias } => record_db.delete_alias(&alias)?,
            AliasCommand::Rename { alias, new } => {
                record_db.rename_alias(&alias, &new)?;
            }
        },
        Command::Get { citation_keys } => {
            // Collect all entries which are not null
            let valid_entries = validate_and_retrieve(
                citation_keys.iter().map(|s| s as &str),
                &mut record_db,
                &client,
            );
            // print biblatex strings
            print_records(valid_entries)
        }
        Command::Source { paths, file_type } => {
            let mut buffer = Vec::new();
            let mut citation_keys = HashSet::new();

            for path in paths {
                buffer.clear();
                let mut f = File::open(path.clone()).with_context(|| {
                    format!("Source file '{}' could not be opened.", path.display())
                })?;
                f.read_to_end(&mut buffer)?;

                get_citekeys(
                    file_type.unwrap_or(guess_source_file_type(&path)?),
                    &buffer,
                    &mut citation_keys,
                )
            }

            let valid_entries = validate_and_retrieve(
                citation_keys.iter().map(|s| s as &str),
                &mut record_db,
                &client,
            );

            print_records(valid_entries)
        }
        Command::Show => todo!(),
    };

    record_db.optimize()?;

    Ok(())
}

/// Iterate over records, printing the entries and warning about duplicates.
fn print_records(records: HashMap<RemoteId, Vec<KeyedEntry>>) {
    for (canonical, entry_vec) in records.iter() {
        if entry_vec.len() > 1 {
            warn!(
                "Multiple keys for `{canonical}`: {}",
                entry_vec.iter().map(|e| &e.key).join(", ")
            );
        }
        for record in entry_vec {
            println!("{record}");
        }
    }
}

/// Validate and retrieve records.
fn validate_and_retrieve<'a, T: Iterator<Item = &'a str>>(
    citation_keys: T,
    record_db: &mut RecordDatabase,
    client: &HttpClient,
) -> HashMap<RemoteId, Vec<KeyedEntry>> {
    let mut records: HashMap<RemoteId, Vec<KeyedEntry>> = HashMap::new();

    for (record, canonical) in citation_keys
        .map(RecordId::from)
        .filter_map(|citation_key| {
            get_record(record_db, citation_key, client).map_or_else(
                |err| {
                    error!("{err}");
                    None
                },
                Some,
            )
        })
    {
        records.entry(canonical).or_default().push(record)
    }
    records
}
