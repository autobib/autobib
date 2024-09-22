pub mod cite_search;
pub mod db;
mod entry;
pub mod error;
mod http;
pub mod provider;
mod record;
pub mod term;

use std::{
    collections::{
        btree_map::Entry::{Occupied, Vacant},
        BTreeMap, HashSet,
    },
    fs::{create_dir_all, File},
    io::{self, Read},
    path::PathBuf,
    str::FromStr,
    thread,
};

use anyhow::Result;
use chrono::{DateTime, Local};
use clap::{Parser, Subcommand};
use clap_verbosity_flag::{Verbosity, WarnLevel};
use etcetera::{choose_app_strategy, AppStrategy, AppStrategyArgs};
use itertools::Itertools;
use log::{error, info, warn};
use nonempty::NonEmpty;
use nucleo_picker::Picker;
use term::{Editor, EditorConfig};

use self::{
    cite_search::{get_citekeys, SourceFileType},
    db::{CitationKey, EntryData, RawRecordData, RecordData, RecordDatabase},
};
pub use self::{
    entry::Entry,
    http::HttpClient,
    record::{get_record, Alias, RecordId, RemoteId},
};

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
        /// Fields to search (e.g. author, title).
        #[clap(short, long, value_delimiter = ',')]
        fields: Vec<String>,
    },
    /// Retrieve records given citation keys.
    #[command(alias = "g")]
    Get {
        /// The citation keys to retrieve.
        citation_keys: Vec<String>,
    },
    /// Create or edit a local record with the given handle.
    #[command(alias = "l")]
    Local { handle: String },
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
    /// Edit existing records.
    #[command(alias = "u")]
    Util {
        #[command(subcommand)]
        util_command: UtilCommand,
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

/// Manage aliases.
#[derive(Subcommand)]
enum UtilCommand {
    /// Add a new alias.
    Check,
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
        error!("{err}");
    }
}

/// Run the CLI.
fn run_cli(cli: Cli) -> Result<()> {
    info!("SQLite version: {}", rusqlite::version());
    info!("Database format version: {}", db::version());

    // Open or create the database
    let mut record_db = if let Some(db_path) = cli.database {
        // at a user-provided path
        info!("Using user-provided database file '{}'", db_path.display());

        RecordDatabase::open(db_path)?
    } else {
        // at the default path
        let strategy = choose_app_strategy(AppStrategyArgs {
            top_level_domain: "org".to_owned(),
            author: env!("CARGO_PKG_NAME").to_owned(),
            app_name: env!("CARGO_PKG_NAME").to_owned(),
        })?;

        let data_dir = strategy.data_dir();

        create_dir_all(&data_dir)?;
        let default_db_path = data_dir.join("records.db");
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
                record_db.delete_alias(&alias)?;
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

            let editor = Editor::new(EditorConfig { suffix: ".bib" });

            if let Some(new_entry) = editor.edit(&entry)? {
                let Entry {
                    key: new_key,
                    record_data: new_record_data,
                } = new_entry;

                if new_key != entry.key() {
                    let alias = Alias::from_str(&new_key)?;
                    record_db.insert_alias(&alias, &canonical)?;
                }

                if new_record_data != *entry.data() {
                    record_db.update_cached_data(&canonical, &new_record_data)?;
                }
            }
        }
        Command::Local { handle } => {
            let remote_id = RemoteId::local(&handle);

            record_db
                .get_cached_data_or_set_default(&remote_id, || (&RecordData::default()).into())?;
        }
        Command::Get { citation_keys } => {
            // Collect all entries which are not null
            let valid_entries = validate_and_retrieve(
                citation_keys.iter().map(|s| s as &str),
                &mut record_db,
                &client,
            );

            // print biblatex strings
            print_records(valid_entries);
        }
        Command::Find { fields } => {
            let fields_to_search: HashSet<String> =
                fields.iter().map(|f| f.to_lowercase()).collect();

            if let Some(res) = choose_canonical_id(record_db, fields_to_search)? {
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
                    Ok(_) => {
                        if let Some(mode) = file_type.or_else(|| {
                            SourceFileType::detect(&path).map_or_else(
                                |err| {
                                    error!(
                                        "File '{}': {err}. Force filetype with `--file-type`.",
                                        path.display()
                                    );
                                    None
                                },
                                Some,
                            )
                        }) {
                            info!("Reading citation keys from '{}'", path.display());
                            get_citekeys(mode, &buffer, &mut container);
                            buffer.clear();
                        }
                    }
                    Err(err) => error!(
                        "Failed to read contents of path '{}': {err}",
                        path.display()
                    ),
                };
            }

            let valid_entries =
                validate_and_retrieve(container.iter().map(|s| s as &str), &mut record_db, &client);

            print_records(valid_entries);
        }
        Command::Show => todo!(),
        Command::Util { util_command } => match util_command {
            UtilCommand::Check => {
                info!("Validating record binary data");
                record_db.validate_record_data()?;
                info!("Validating internal database consistency");
                record_db.validate_consistency()?;
                info!("Checking for dangling records");
                record_db.validate_record_indexing()?;
            }
        },
    };

    Ok(())
}

/// Create a field filter renderer, which given a set of allowed fields renders those fields which
/// are present in the data in alphabetical order, separated by the `separator`.
fn field_filter_renderer(
    fields_to_search: HashSet<String>,
    separator: &'static str,
) -> impl Fn(RawRecordData, &RemoteId, DateTime<Local>) -> String {
    move |data, _, _| {
        let field_string = data
            .fields()
            .filter(|(key, _)| fields_to_search.contains(*key))
            .map(|(_, val)| val)
            .join(separator);
        format!("{}: {field_string}", data.entry_type())
    }
}

/// Open an interactive prompt for the user to select a record.
fn choose_canonical_id(
    mut record_db: RecordDatabase,
    fields_to_search: HashSet<String>,
) -> Result<Option<RemoteId>, io::Error> {
    // initialize picker
    let mut picker = Picker::default();

    // populate the picker from a separate thread
    let injector = picker.injector();
    thread::spawn(move || {
        record_db.inject_all_records(injector, field_filter_renderer(fields_to_search, " ~ "))
    });

    // get the selection
    picker.pick().map(Option::<&_>::cloned)
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
                entries.iter().map(Entry::key).join(", ")
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
