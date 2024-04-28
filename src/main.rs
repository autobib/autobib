mod database;
mod entry;
mod record;
mod source;

use std::path::PathBuf;
use std::str::FromStr;

use clap::{Parser, Subcommand};
use xdg::BaseDirectories;

use database::RecordDatabase;
use entry::KeyedEntry;
use record::*;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(long)]
    database: Option<PathBuf>,

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
    /// Generate records from sourc(es).
    #[command(alias = "s")]
    Source,
    /// Show metadata for citation key.
    #[command()]
    Show,
}

#[derive(Subcommand)]
enum AliasCommand {
    Add,
    Delete,
    Rename,
}

fn main() {
    let cli = Cli::parse();

    // Open or create the database
    let record_db = if let Some(db_path) = cli.database {
        // at a user-provided path
        RecordDatabase::open_or_create(&db_path).expect("Failed to open or create database.")
    } else {
        // at the default path
        let xdg_dirs =
            BaseDirectories::with_prefix("autobib").expect("Could not find valid base directory.");
        RecordDatabase::open_or_create(
            xdg_dirs
                .place_data_file("cache.db")
                .expect("Failed to create data directory."),
        )
        .expect("Failed to open or create database.")
    };

    match cli.command {
        Command::Alias { alias_command: cmd } => match cmd {
            AliasCommand::Add => todo!(),
            AliasCommand::Delete => todo!(),
            AliasCommand::Rename => todo!(),
        },
        Command::Get { citation_keys } => {
            // Collect all entries which are not null
            let valid_entries =
                validate_and_retrieve(citation_keys.iter().map(|s| s as &str), record_db);
            // print biblatex strings
            for entry in valid_entries {
                // TODO: replace me when serde_bibtex serialize is implemented
                println!("{}", entry)
            }
        }
        Command::Source => todo!(),
        Command::Show => todo!(),
    }
}

/// Validate and retrieve records.
///
/// - During validation, filter invalid citation keys and print error messages.
/// - During retrieval, filter null records and print error messages.
fn validate_and_retrieve<'a, T: Iterator<Item = &'a str>>(
    citation_keys: T,
    mut record_db: RecordDatabase,
) -> Vec<KeyedEntry> {
    citation_keys
        // parse the source:sub_id arguments and perform cheap validation
        .filter_map(|input| match CitationKeyInput::from_str(input) {
            Ok(record_id) => Some(record_id),
            Err(err) => {
                eprintln!("{err}");
                None
            }
        })
        // retrieve records
        .filter_map(|citation_key| {
            get_record(&mut record_db, &citation_key).map_or_else(
                // error retrieving record
                |err| {
                    eprintln!("{err}");
                    None
                },
                |response| match response {
                    Some(entry) => Some(KeyedEntry {
                        key: citation_key,
                        contents: entry,
                    }),
                    None => {
                        eprintln!("'null record: {citation_key}'");
                        None
                    }
                },
            )
        })
        .collect()
}
