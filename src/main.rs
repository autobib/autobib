mod citekey;
pub mod database;
mod entry;
pub mod error;
mod record;
pub mod source;

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use xdg::BaseDirectories;

use citekey::tex::get_citekeys;
pub use database::{CitationKey, RecordDatabase};
use entry::KeyedEntry;
pub use record::{get_record, Alias, RecordId, RemoteId};

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
    /// Show metadata for citation key.
    #[command()]
    Show,
    /// Generate records from source(s).
    #[command(alias = "s")]
    Source { paths: Vec<PathBuf> },
}

#[derive(Subcommand)]
enum AliasCommand {
    Add { alias: Alias, target: RecordId },
    Delete { alias: Alias },
    Rename { alias: Alias, new: Alias },
}

// TODO: replace this with a proper error handling mechanism
fn fail_on_err<T, E: fmt::Display>(result: Result<T, E>) -> T {
    result.unwrap_or_else(|e| {
        eprintln!("{e}");
        process::exit(1)
    })
}

fn main() {
    let cli = Cli::parse();

    // Open or create the database
    let mut record_db = if let Some(db_path) = cli.database {
        // at a user-provided path
        RecordDatabase::open(db_path).expect("Failed to open database.")
    } else {
        // at the default path
        // TODO: properly handle errors
        let xdg_dirs =
            BaseDirectories::with_prefix("autobib").expect("Could not find valid base directory.");
        RecordDatabase::open(
            xdg_dirs
                .place_data_file("cache.db")
                .expect("Failed to create data directory."),
        )
        .expect("Failed to open or create database.")
    };

    match cli.command {
        Command::Alias { alias_command } => match alias_command {
            AliasCommand::Add { alias, target } => {
                // first retrieve 'target', in case it does not yet exist in the database
                // fail_on_err(get_record(&mut record_db, &target));
                // then link to it
                fail_on_err(record_db.insert_alias(&alias, &target));
            }
            // TODO: deletion fails silently if the alias does not exist
            AliasCommand::Delete { alias } => fail_on_err(record_db.delete_alias(&alias)),
            AliasCommand::Rename { alias, new } => {
                fail_on_err(record_db.rename_alias(&alias, &new))
            }
        },
        Command::Get { citation_keys } => {
            // Collect all entries which are not null
            let valid_entries =
                validate_and_retrieve(citation_keys.iter().map(|s| s as &str), record_db);
            // print biblatex strings
            print_records(valid_entries)
        }
        Command::Source { paths } => {
            let mut buffer = Vec::new();
            let mut citation_keys = HashSet::new();
            for path in paths {
                buffer.clear();
                match path.extension().and_then(OsStr::to_str) {
                    Some("tex") => {
                        // TODO: proper file error handling
                        let mut f = File::open(path).unwrap();
                        f.read_to_end(&mut buffer).unwrap();
                        get_citekeys(&buffer, &mut citation_keys);
                    }
                    Some(ext) => {
                        eprintln!("Error: File type '{ext}' not supported");
                        process::exit(1)
                    }
                    None => {
                        eprintln!("Error: File type required");
                        process::exit(1)
                    }
                }
            }
            let valid_entries =
                validate_and_retrieve(citation_keys.iter().map(|s| s as &str), record_db);
            // print biblatex strings
            print_records(valid_entries)
        }
        Command::Show => todo!(),
    }
}

/// Iterate over records, printing the entries and warning about duplicates.
fn print_records(records: HashMap<RemoteId, Vec<KeyedEntry>>) {
    for (canonical, entry_vec) in records.iter() {
        if entry_vec.len() > 1 {
            // TODO: better printing
            eprint!("Duplicate keys for '{canonical}':");
            for entry in entry_vec.iter() {
                eprint!(" '{}'", entry.key);
            }
            eprintln!();
        }
        for record in entry_vec {
            println!("{record}");
        }
    }
}

/// Validate and retrieve records.
fn validate_and_retrieve<'a, T: Iterator<Item = &'a str>>(
    citation_keys: T,
    mut record_db: RecordDatabase,
) -> HashMap<RemoteId, Vec<KeyedEntry>> {
    let mut records: HashMap<RemoteId, Vec<KeyedEntry>> = HashMap::new();

    for (record, canonical) in citation_keys
        .map(RecordId::from)
        .filter_map(|citation_key| {
            get_record(&mut record_db, citation_key).map_or_else(
                |err| {
                    eprintln!("{err}");
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
