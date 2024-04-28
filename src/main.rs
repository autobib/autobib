mod database;
mod entry;
mod record;
mod source;

use std::path::PathBuf;
use std::str::FromStr;

use clap::{Parser, Subcommand};
use rusqlite::Result;

use database::RecordDatabase;
use entry::KeyedEntry;
use record::*;

// TODO: Replace with XDG
const DATABASE_FILE: &str = "cache.db";

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
    Alias,
    /// Retrieve records given citation keys.
    #[command(alias = "g")]
    Get {
        /// The citation keys to retrieve.
        citation_keys: Vec<String>,
    },
    /// Generate records from sourc(es).
    #[command(alias = "s")]
    Source,
}

fn main() {
    let cli = Cli::parse();

    // Open or create the database
    let record_db = if let Some(db_path) = cli.database {
        // at a user-provided path
        RecordDatabase::open_or_create(&db_path).expect("Failed to open or create database.")
    } else {
        // at the default path
        RecordDatabase::open_or_create(DATABASE_FILE).expect("Failed to open or create database.")
    };

    match cli.command {
        Command::Alias => {
            eprintln!("Alias command not implemented.");
        }
        Command::Get { citation_keys } => {
            // Collect all entries which are not null
            let valid_entries =
                validate_and_retrieve(citation_keys.iter().map(|s| s as &str), record_db);
            // print biblatex strings
            for entry in valid_entries {
                println!("{}", entry)
            }
        }
        Command::Source => {
            eprintln!("Auto command not implemented.");
        }
    }
}

/// Validate and retrieve records.
/// During validation, invalid citation keys are filtered out and error messages are printed.
/// During retrieval, records are fetched from the database, with null records filtered out, and error messages are printed;
fn validate_and_retrieve<'a, T: Iterator<Item = &'a str>>(
    citation_keys: T,
    mut record_db: RecordDatabase,
) -> Vec<KeyedEntry> {
    citation_keys
        // parse the source:sub_id arguments and perform cheap validation
        .filter_map(|input| match CitationKey::from_str(input) {
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

/// Populate the database with some records for testing purposes.
fn create_test_db() -> Result<RecordDatabase, RecordError> {
    use entry::{Entry, Fields};
    match std::fs::remove_file(DATABASE_FILE) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => panic!("Testing database file has been overwritten!"),
    }

    let mut record_db = RecordDatabase::create(DATABASE_FILE)?;

    let entry_1 = Entry {
        entry_type: "code".to_string(),
        fields: Fields {
            author: Some("Rutar, Alex and Wu, Peiran".to_string()),
            title: Some("Autobib".to_string()),
            ..Fields::default()
        },
    };
    record_db.set_cached_data(&RecordId::from_parts("test", "000"), &entry_1, None)?;

    let entry_2 = Entry {
        entry_type: "article".to_string(),
        fields: Fields {
            author: Some("Author, Test".to_string()),
            title: Some("A Sample Paper".to_string()),
            ..Fields::default()
        },
    };
    record_db.set_cached_data(&RecordId::from_parts("test", "002"), &entry_2, None)?;

    Ok(record_db)
}
