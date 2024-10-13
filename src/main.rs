pub mod cite_search;
pub mod db;
mod entry;
pub mod error;
mod http;
mod logger;
pub mod provider;
mod record;
pub mod term;

use std::{
    collections::{
        btree_map::Entry::{Occupied, Vacant},
        BTreeMap, HashSet,
    },
    fs::{create_dir_all, read_to_string, File},
    io::{self, Read},
    path::{Path, PathBuf},
    process::exit,
    str::FromStr,
    thread,
};

use anyhow::{bail, Result};
use chrono::{DateTime, Local};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::aot::{generate, Shell};
use clap_verbosity_flag::{Verbosity, WarnLevel};
use crossterm::tty::IsTty;
use etcetera::{choose_app_strategy, AppStrategy, AppStrategyArgs};
use itertools::Itertools;
use log::{error, info, warn};
use nonempty::NonEmpty;
use nucleo_picker::Picker;
use serde::Serializer as _;
use serde_bibtex::ser::Serializer;
use term::{Editor, EditorConfig};

use self::{
    cite_search::{get_citekeys, SourceFileType},
    db::{
        CitationKey, EntryData, RawRecordData, RecordData, RecordDatabase, RecordsDefaultResponse,
    },
    logger::Logger,
    record::Record,
};
pub use self::{
    entry::Entry,
    http::HttpClient,
    record::{get_record, Alias, RecordId, RemoteId},
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    database: Option<PathBuf>,

    #[command(flatten)]
    verbose: Verbosity<WarnLevel>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage aliases.
    Alias {
        #[command(subcommand)]
        alias_command: AliasCommand,
    },
    /// Generate a shell completions script.
    #[clap(hide = true)]
    Completions {
        /// The shell for which to generate the script.
        shell: Shell,
    },
    /// Edit existing records.
    Edit {
        /// The citation key to edit.
        citation_key: String,
    },
    /// Search for a citation key.
    Find {
        /// Fields to search (e.g. author, title).
        #[clap(short, long, value_delimiter = ',')]
        fields: Vec<String>,
    },
    /// Retrieve records given citation keys.
    Get {
        /// The citation keys to retrieve.
        citation_keys: Vec<String>,
        /// Write output to file.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Create or edit a local record with the given handle.
    Local {
        /// The name for the record.
        id: String,
        /// Edit record.
        #[arg(long, action)]
        edit: bool,
        /// Create local record from bibtex file.
        #[arg(short, long, value_name = "PATH")]
        from: Option<PathBuf>,
    },
    /// Show metadata for citation key.
    Show,
    /// Generate records by searching for citation keys inside files.
    Source {
        /// The files in which to search.
        paths: Vec<PathBuf>,
        /// Override file type detection.
        #[arg(long)]
        file_type: Option<SourceFileType>,
        /// Write output to file.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Utilities to manage database.
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
    #[command(alias = "rm")]
    Delete { alias: Alias },
    /// Rename an existing alias.
    #[command(alias = "mv")]
    Rename { alias: Alias, new: Alias },
}

/// Manage aliases.
#[derive(Subcommand)]
enum UtilCommand {
    /// Check database for errors.
    Check,
}

static LOGGER: Logger = Logger {};

fn main() {
    let cli = Cli::parse();

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

    // run the cli
    if let Err(err) = run_cli(cli) {
        error!("{err}");
    }

    if Logger::has_error() {
        exit(1)
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
        Command::Completions { shell: _ } => {
            unreachable!("Request for completions script should have been handled earlier and the program should have exited then.");
        }
        Command::Edit { citation_key } => {
            let record = get_record(
                &mut record_db,
                RecordId::from(citation_key.as_str()),
                &client,
            )?;

            edit_record_and_update_database(&mut record_db, record)?;
        }
        Command::Local { id, edit, from } => {
            let remote_id = RemoteId::local(&id);

            let data = match record_db
                .get_cached_data_or_set_default(&remote_id, create_default_record(from.as_ref()))?
            {
                RecordsDefaultResponse::Found(raw_record_data, _, _) => {
                    if from.is_some() {
                        bail!("Local record '{id}' already exists")
                    } else {
                        raw_record_data
                    }
                }
                RecordsDefaultResponse::New(raw_record_data) => raw_record_data,
                RecordsDefaultResponse::Failed(err) => bail!(err),
            };

            if edit {
                edit_record_and_update_database(
                    &mut record_db,
                    Record {
                        key: remote_id.to_string(),
                        data,
                        canonical: remote_id,
                    },
                )?;
            }
        }
        Command::Get { citation_keys, out } => {
            // Collect all entries which are not null
            let valid_entries = validate_and_retrieve(
                citation_keys.iter().map(|s| s as &str),
                &mut record_db,
                &client,
            );

            output_records(out.as_ref(), valid_entries)?;
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
        Command::Source {
            paths,
            file_type,
            out,
        } => {
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

            output_records(out.as_ref(), valid_entries)?;
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

fn create_default_record<P: AsRef<Path>>(
    from: Option<P>,
) -> impl FnOnce() -> Result<RawRecordData, anyhow::Error> {
    || {
        Ok(if let Some(path) = from {
            let bibtex = read_to_string(path)?;
            let entry = Entry::<RawRecordData>::from_str(&bibtex)?;
            entry.record_data
        } else {
            (&RecordData::default()).into()
        })
    }
}

fn edit_record_and_update_database(
    record_db: &mut RecordDatabase,
    record: Record,
) -> Result<Entry<RawRecordData>, anyhow::Error> {
    let Record {
        key,
        data,
        canonical,
    } = record;

    let mut entry = Entry::try_new(key, data)?;

    let editor = Editor::new(EditorConfig { suffix: ".bib" });

    if let Some(new_entry) = editor.edit(&entry)? {
        let Entry {
            key: ref new_key,
            record_data: ref new_record_data,
        } = new_entry;

        if new_key != entry.key() {
            let alias = Alias::from_str(new_key)?;
            info!("Creating new alias '{alias}' for '{canonical}'");
            record_db.insert_alias(&alias, &canonical)?;
        }

        if new_record_data != entry.data() {
            info!("Updating cached data for '{canonical}'");
            record_db.update_cached_data(&canonical, new_record_data)?;
        }

        entry = new_entry;
    }

    Ok(entry)
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

/// Either write records to stdout, or to a provided file.
fn output_records<D: EntryData, P: AsRef<Path>>(
    out: Option<P>,
    records: BTreeMap<RemoteId, NonEmpty<Entry<D>>>,
) -> Result<(), serde_bibtex::Error> {
    if let Some(path) = out {
        let writer = io::BufWriter::new(std::fs::File::create(path)?);
        write_records(writer, records)?;
    } else {
        let stdout = io::stdout();
        if stdout.is_tty() {
            // do not write an extra newline if interactive
            if !records.is_empty() {
                write_records(stdout, records)?;
            }
        } else {
            let writer = io::BufWriter::new(stdout);
            write_records(writer, records)?;
        }
    };

    Ok(())
}

/// Iterate over records, writing the entries and warning about duplicates.
fn write_records<W: io::Write, D: EntryData>(
    writer: W,
    records: BTreeMap<RemoteId, NonEmpty<Entry<D>>>,
) -> Result<(), serde_bibtex::Error> {
    let mut serializer = Serializer::unchecked(writer);

    serializer.collect_seq(records.iter().flat_map(|(canonical, entries)| {
        if entries.len() > 1 {
            warn!(
                "Multiple keys for '{canonical}': {}",
                entries.iter().map(Entry::key).join(", ")
            );
        };
        entries
    }))
}

/// Validate and retrieve records.
fn validate_and_retrieve<'a, T: Iterator<Item = &'a str>>(
    citation_keys: T,
    record_db: &mut RecordDatabase,
    client: &HttpClient,
) -> BTreeMap<RemoteId, NonEmpty<Entry<RawRecordData>>> {
    let mut records: BTreeMap<RemoteId, NonEmpty<Entry<RawRecordData>>> = BTreeMap::new();

    for (bibtex_entry, canonical) in citation_keys
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
        .filter_map(|record| {
            let Record {
                key,
                data,
                canonical,
            } = record;
            Entry::try_new(key, data).map_or_else(
                |err| {
                    error!("{err}\n  Suggested fix: use an alias which does not contain disallowed characters: {{}}(),=\\#%\"");
                    None
                },
                |entry| Some((entry, canonical)),
            )
        })
    {
        match records.entry(canonical) {
            Occupied(e) => e.into_mut().push(bibtex_entry),
            Vacant(e) => {
                e.insert(NonEmpty::singleton(bibtex_entry));
            }
        }
    }
    records
}
