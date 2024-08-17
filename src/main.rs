pub mod cite_search;
pub mod db;
mod entry;
pub mod error;
mod http;
pub mod provider;
mod record;
pub mod term;

use std::collections::{
    btree_map::Entry::{Occupied, Vacant},
    BTreeMap, HashSet,
};
use std::fs::{create_dir_all, File};
use std::io::{self, Read};
use std::path::PathBuf;
use std::str::FromStr;
use std::thread;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Local};
use clap::{Parser, Subcommand};
use clap_verbosity_flag::{Verbosity, WarnLevel};
use directories::ProjectDirs;
use itertools::Itertools;
use log::{error, info, warn};
use nonempty::NonEmpty;
use term::{Config, Editor};

use self::cite_search::{get_citekeys, SourceFileType};
use self::db::{CitationKey, EntryData, RawRecordData, RecordDatabase};
pub use self::entry::Entry;
pub use self::http::HttpClient;
pub use self::record::{get_record, Alias, RecordId, RemoteId};
use self::term::Picker;

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
    /// Edit existing records.
    #[command(alias = "e")]
    Edit {
        /// The citation key to edit.
        citation_key: String,
    },
    /// Search for a citation key.
    #[command(alias = "f")]
    Find {
        #[clap(short, long)]
        field: Vec<String>,
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

/// Manage aliases.
#[derive(Subcommand)]
enum AliasCommand {
    /// Add a new alias.
    Add { alias: Alias, target: RecordId },
    /// Delete an existing alias.
    Delete { alias: Alias },
    /// Rename an existing alias.
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

/// Run the CLI.
fn run_cli(cli: Cli) -> Result<()> {
    // Initialize project directory.
    let proj_dirs = match ProjectDirs::from("com", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_NAME")) {
        Some(p) => p,
        None => return Err(anyhow!("Failed to get project working directory.")),
    };

    info!("SQLite version: {}", rusqlite::version());
    info!("Database format version: {}", db::version());

    // Open or create the database
    let mut record_db = if let Some(db_path) = cli.database {
        // at a user-provided path
        info!("Using user-provided database file '{}'", db_path.display());

        RecordDatabase::open(db_path)?
    } else {
        // at the default path
        create_dir_all(proj_dirs.data_dir())?;
        let default_db_path = proj_dirs.data_dir().join("records.db");
        info!(
            "Using default database file '{}'",
            default_db_path.display()
        );

        RecordDatabase::open(default_db_path)?
    };

    // Initialize the reqwest Client
    let client = HttpClient::new()?;

    // Run the cli
    match cli.command {
        Command::Alias { alias_command } => match alias_command {
            AliasCommand::Add { alias, target } => {
                info!("Creating alias '{alias}' for '{target}'");
                // first retrieve 'target', in case it does not yet exist in the database
                get_record(&mut record_db, target.clone(), &client)?;
                // then link to it
                record_db.insert_alias(&alias, &target)?;
            }
            AliasCommand::Delete { alias } => {
                info!("Deleting alias '{alias}'");
                record_db.delete_alias(&alias)?
            }
            AliasCommand::Rename { alias, new } => {
                info!("Rename alias '{alias}' to '{new}'");
                record_db.rename_alias(&alias, &new)?;
            }
        },
        Command::Edit { citation_key } => {
            let (entry, canonical) = get_record(
                &mut record_db,
                RecordId::from(citation_key.as_str()),
                &client,
            )?;

            let editor = Editor::new(Config { suffix: ".bib" });

            if let Some(new_entry) = editor.edit(&entry)? {
                let (new_key, new_record_data) = new_entry.into_parts();

                if new_key != entry.key() {
                    let alias = Alias::from_str(&new_key)?;
                    record_db.insert_alias(&alias, &canonical)?
                }

                if new_record_data != *entry.data() {
                    record_db.update_cached_data(&canonical, new_record_data)?;
                }
            }
        }
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
        Command::Find { field } => {
            let allowed_fields: HashSet<String> = field.iter().map(|f| f.to_lowercase()).collect();

            if let Some(res) = choose_canonical_id(record_db, allowed_fields)? {
                println!("{res}");
            } else {
                error!("No item selected.");
            }
        }
        Command::Source { paths, file_type } => {
            let mut buffer = Vec::new();

            // The citation keys do not need to be sorted since sorting
            // happens in the `validate_and_retrieve` function.
            let mut container = HashSet::new();

            for path in paths {
                match File::open(path.clone()).and_then(|mut f| f.read_to_end(&mut buffer)) {
                    Ok(_) => match SourceFileType::detect(&path) {
                        Ok(mode) => {
                            info!("Reading citation keys from '{}'", path.display());
                            get_citekeys(file_type.unwrap_or(mode), &buffer, &mut container);
                            buffer.clear();
                        }
                        Err(err) => error!(
                            "File '{}': {err}. Force filetype with `--file-type`.",
                            path.display()
                        ),
                    },
                    Err(err) => error!(
                        "Failed to read contents of path '{}': {err}",
                        path.display()
                    ),
                };
            }

            let valid_entries =
                validate_and_retrieve(container.iter().map(|s| s as &str), &mut record_db, &client);

            print_records(valid_entries)
        }
        Command::Show => todo!(),
    };

    Ok(())
}

/// Create a field filter renderer, which given a set of allowed fields renders those fields which
/// are present in the data in alphabetical order, separated by the `separator`.
fn field_filter_renderer(
    allowed_fields: HashSet<String>,
    separator: &'static str,
) -> impl Fn(RawRecordData, &RemoteId, DateTime<Local>) -> String {
    move |data, _, _| {
        let field_string = data
            .fields()
            .filter(|(key, _)| allowed_fields.contains(*key))
            .map(|(_, val)| val)
            .join(separator);
        format!("{}: {field_string}", data.entry_type())
    }
}

/// Open an interactive prompt for the user to select a record.
fn choose_canonical_id(
    mut record_db: RecordDatabase,
    allowed_fields: HashSet<String>,
) -> Result<Option<RemoteId>, io::Error> {
    // initialize matcher
    let mut picker = Picker::new(1);

    // populate the matcher from a separate thread
    let injector = picker.injector();
    thread::spawn(move || {
        record_db.inject_all_records(field_filter_renderer(allowed_fields, " ~ "), injector)
    });

    // get the selection
    picker.pick().map(|opt| opt.cloned())
}

/// Iterate over records, printing the entries and warning about duplicates.
///
/// TODO: replace this with a `write_records` method and an abstract writer.
/// TODO: replace the `records` struct with a custom wrapper struct.
fn print_records<D: EntryData>(records: BTreeMap<RemoteId, NonEmpty<Entry<D>>>) {
    for (canonical, entries) in records.iter() {
        if entries.len() > 1 {
            warn!(
                "Multiple keys for '{canonical}': {}",
                entries.iter().map(|e| e.key()).join(", ")
            );
        }
        for record in entries {
            println!("{record}");
        }
    }
}

/// Validate and retrieve records.
fn validate_and_retrieve<'a, T: Iterator<Item = &'a str>>(
    citation_keys: T,
    record_db: &mut RecordDatabase,
    client: &HttpClient,
) -> BTreeMap<RemoteId, NonEmpty<Entry<RawRecordData>>> {
    let mut records: BTreeMap<RemoteId, NonEmpty<Entry<RawRecordData>>> = BTreeMap::new();

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
        match records.entry(canonical) {
            Occupied(entry) => entry.into_mut().push(record),
            Vacant(entry) => {
                entry.insert(NonEmpty::singleton(record));
            }
        }
    }
    records
}
