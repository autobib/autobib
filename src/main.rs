mod database;
mod entry;
mod record;
mod source;

use std::fmt;
use std::path::PathBuf;
use std::process;
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

/// Parse an argument using a [`FromStr`] implementation.
fn from_str_parser<T: FromStr>(s: &str) -> Result<T, String>
where
    <T as FromStr>::Err: fmt::Display,
{
    T::from_str(s).map_err(|e| e.to_string())
}

#[derive(Subcommand)]
enum AliasCommand {
    Add {
        #[arg(value_parser = from_str_parser::<Alias>)]
        alias: Alias,
        #[arg(value_parser = from_str_parser::<CitationKeyInput>)]
        target: CitationKeyInput,
    },
    Delete {
        #[arg(value_parser = from_str_parser::<Alias>)]
        alias: Alias,
    },
    Rename {
        #[arg(value_parser = from_str_parser::<Alias>)]
        alias: Alias,
        #[arg(value_parser = from_str_parser::<Alias>)]
        new: Alias,
    },
}

fn fail_on_err<T, E: fmt::Display>(err: Result<T, E>) {
    match err {
        Err(why) => {
            eprintln!("{why}");
            process::exit(1)
        }
        _ => (),
    }
}

fn main() {
    let cli = Cli::parse();

    // Open or create the database
    let mut record_db = if let Some(db_path) = cli.database {
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
        Command::Alias { alias_command } => match alias_command {
            AliasCommand::Add { alias, target } => {
                // first retrieve 'target', in case it does not yet exist in the database
                fail_on_err(get_record(&mut record_db, &target));
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
                        eprintln!("Null record '{citation_key}'");
                        None
                    }
                },
            )
        })
        .collect()
}
