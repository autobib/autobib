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
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::aot::{generate, Shell};
use clap_verbosity_flag::{Verbosity, WarnLevel};
use crossterm::tty::IsTty;
use etcetera::{choose_app_strategy, AppStrategy, AppStrategyArgs};
use itertools::Itertools;
use log::{error, info, warn};
use nonempty::NonEmpty;
use nucleo_picker::Picker;
use serde::Serializer as _;
use serde_bibtex::{ser::Serializer, validate::is_entry_key};
use term::{Confirm, Editor, EditorConfig};

use self::{
    cite_search::{get_citekeys, SourceFileType},
    db::{
        row::{self, DatabaseEntry, Row},
        CitationKey, EntryData, RawRecordData, RecordData, RecordDatabase, RowData,
    },
    logger::Logger,
    record::{
        get_remote_record, GetRecordEntryResponse, GetRecordResponse, GetRemoteRecordResponse,
        Record,
    },
};
pub use self::{
    entry::Entry,
    http::HttpClient,
    record::{get_record, get_record_entry, Alias, RecordId, RemoteId},
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Use record database.
    #[arg(short, long, value_name = "PATH")]
    database: Option<PathBuf>,

    #[command(flatten)]
    verbose: Verbosity<WarnLevel>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Copy, Clone, ValueEnum, Default)]
enum InfoReportType {
    #[default]
    All,
    Canonical,
    Valid,
    Equivalent,
    Modified,
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
    Delete {
        /// The citation key to delete.
        citation_key: RecordId,
        /// Delete without prompting.
        #[arg(short, long)]
        force: bool,
    },
    /// Edit existing records.
    Edit {
        /// The citation key to edit.
        citation_key: RecordId,
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
        citation_keys: Vec<RecordId>,
        /// Write output to file.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Ignore null records and aliases.
        #[arg(long)]
        ignore_null: bool,
    },
    /// Show metadata for citation key.
    Info {
        /// The citation key to show info.
        citation_key: RecordId,
        /// The type of information to display.
        #[arg(value_enum, default_value_t = InfoReportType::default())]
        report: InfoReportType,
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
        /// Ignore null records and aliases.
        #[arg(long)]
        ignore_null: bool,
    },
    /// Update the data associated with an existing citation key.
    Update {
        /// The citation key to update.
        citation_key: RecordId,
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
    Add {
        /// The new alias to create.
        alias: Alias,
        /// What the alias points to.
        target: RecordId,
    },
    /// Delete an existing alias.
    #[command(alias = "rm")]
    Delete {
        /// The new alias to delete.
        alias: Alias,
    },
    /// Rename an existing alias.
    #[command(alias = "mv")]
    Rename {
        /// The name of the existing alias.
        alias: Alias,
        /// The name of the new alias.
        new: Alias,
    },
}

/// Manage aliases.
#[derive(Subcommand)]
enum UtilCommand {
    /// Check database for errors.
    Check,
    /// List all valid keys.
    List {
        #[arg(short, long)]
        canonical: bool,
    },
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
    let builder = HttpClient::default_builder();
    let client = HttpClient::new(builder)?;

    // Run the cli
    match cli.command {
        Command::Alias { alias_command } => match alias_command {
            AliasCommand::Add { alias, target } => {
                info!("Creating alias '{alias}' for '{target}'");
                let entry = record_db.entry(&target)?;
                match get_record_entry(entry, target, &client)? {
                    GetRecordEntryResponse::Exists(_, row) => {
                        if !row.apply(row::add_alias(&alias))? {
                            bail!("Alias already exists: '{alias}'")
                        }
                        row.commit()?;
                    }
                    GetRecordEntryResponse::NullRemoteId(remote_id, missing) => {
                        missing.commit()?;
                        error!("Cannot create alias for null record '{remote_id}'");
                    }
                    GetRecordEntryResponse::NullAlias(alias, missing) => {
                        missing.commit()?;
                        error!("Cannot create alias for missing alias '{alias}'");
                    }
                }
            }
            AliasCommand::Delete { alias } => {
                info!("Deleting alias '{alias}'");
                match record_db.delete_alias(&alias)? {
                    db::DeleteAliasResult::Deleted => {}
                    db::DeleteAliasResult::Missing => {
                        bail!("Could not delete alias which does not exist: '{alias}'")
                    }
                }
            }
            AliasCommand::Rename { alias, new } => {
                info!("Rename alias '{alias}' to '{new}'");
                match record_db.rename_alias(&alias, &new)? {
                    db::RenameAliasResult::Renamed => {}
                    db::RenameAliasResult::TargetExists => {
                        bail!("Citation key already exists: '{new}'");
                    }
                }
            }
        },
        Command::Completions { shell: _ } => {
            unreachable!("Request for completions script should have been handled earlier and the program should have exited then.");
        }
        Command::Delete {
            citation_key,
            force,
        } => {
            match record_db.entry(&citation_key)? {
                DatabaseEntry::Exists(row) => {
                    if !force {
                        let referencing = row.apply(row::get_referencing_keys)?;

                        // there are multiple associated keys, prompt before deletion
                        if referencing.len() > 1 {
                            warn!("Deleting this record will delete associated keys:");
                            for key in referencing.iter() {
                                eprintln!("  {key}");
                            }
                            let prompt = Confirm::new("Delete anyway?", false);
                            if !prompt.confirm()? {
                                bail!("Aborted deletion");
                            }
                        }
                    }

                    row.delete()?.commit()?;
                }
                DatabaseEntry::Missing(missing) => {
                    missing.commit()?;
                    bail!("Citation key '{citation_key}' does not exist.");
                }
            }
        }
        Command::Edit { citation_key } => {
            let entry = record_db.entry(&citation_key)?;
            match get_record_entry(entry, citation_key, &client)? {
                record::GetRecordEntryResponse::Exists(record, row) => {
                    edit_record_and_update(row, record)?;
                }
                record::GetRecordEntryResponse::NullRemoteId(remote_id, missing) => {
                    missing.commit()?;
                    error!("Cannot edit null record '{remote_id}'");
                }
                record::GetRecordEntryResponse::NullAlias(alias, missing) => {
                    missing.commit()?;
                    error!("Cannot edit undefined alias '{alias}'");
                }
            }
        }
        Command::Local { id, edit, from } => {
            let remote_id = RemoteId::local(&id);

            let (row, data) = match record_db.entry(&remote_id)? {
                DatabaseEntry::Exists(row) => {
                    if from.is_some() {
                        row.commit()?;
                        bail!("Local record '{id}' already exists")
                    } else {
                        let raw_record_data = row.apply(row::get_row_data)?.data;
                        (row, raw_record_data)
                    }
                }
                DatabaseEntry::Missing(missing) => {
                    let data = if let Some(path) = from {
                        let bibtex = read_to_string(path)?;
                        let entry = Entry::<RawRecordData>::from_str(&bibtex)?;
                        entry.record_data
                    } else {
                        (&RecordData::default()).into()
                    };

                    let row = missing.insert_and_ref(&data, &remote_id)?;
                    (row, data)
                }
            };

            if edit {
                edit_record_and_update(
                    row,
                    Record {
                        key: remote_id.to_string(),
                        data,
                        canonical: remote_id,
                    },
                )?;
            } else {
                row.commit()?;
            }
        }
        Command::Get {
            mut citation_keys,
            out,
            ignore_null,
        } => {
            // Collect all entries which are not null
            let valid_entries = validate_and_retrieve(
                citation_keys.drain(..),
                &mut record_db,
                &client,
                ignore_null,
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
            ignore_null,
        } => {
            let mut buffer = Vec::new();

            // The citation keys do not need to be sorted since sorting
            // happens in the `validate_and_retrieve` function.
            let mut container: HashSet<RecordId> = HashSet::new();

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
                validate_and_retrieve(container.drain(), &mut record_db, &client, ignore_null);

            output_records(out.as_ref(), valid_entries)?;
        }
        Command::Info {
            citation_key,
            report,
        } => match record_db.entry(&citation_key)? {
            DatabaseEntry::Exists(row) => match report {
                InfoReportType::All => {
                    let row_data = row.apply(row::get_row_data)?;
                    println!("Canonical: {}", row_data.canonical);
                    println!(
                        "Equivalent references: {}",
                        row.apply(row::get_referencing_keys)?.iter().join(", ")
                    );
                    println!(
                        "Valid bibtex? {}",
                        if is_entry_key(citation_key.name()) {
                            "yes"
                        } else {
                            "no"
                        }
                    );
                    println!("Data last modified: {}", row_data.modified);
                }
                InfoReportType::Canonical => {
                    println!("{}", row.apply(row::get_canonical)?);
                }

                InfoReportType::Valid => {
                    if !is_entry_key(citation_key.name()) {
                        bail!("Invalid bibtex: {}", citation_key.name());
                    }
                }
                InfoReportType::Equivalent => {
                    for re in row.apply(row::get_referencing_keys)? {
                        println!("{re}");
                    }
                }
                InfoReportType::Modified => {
                    println!("{}", row.apply(row::last_modified)?);
                }
            },
            DatabaseEntry::Missing(missing) => {
                missing.commit()?;
                bail!("Citation key '{citation_key}' does not exist.");
            }
        },
        Command::Update { citation_key } => match record_db.entry(&citation_key)? {
            DatabaseEntry::Exists(row) => {
                let RowData {
                    data: existing_raw_data,
                    canonical,
                    ..
                } = row.apply(row::get_row_data)?;
                match get_remote_record(canonical, &client)? {
                    GetRemoteRecordResponse::Exists(new_raw_data) => {
                        let mut new_record = RecordData::from(new_raw_data);
                        new_record.try_merge(existing_raw_data)?;
                        row.apply(row::update_row_data(&(&new_record).into()))?;
                        row.commit()?;
                    }
                    GetRemoteRecordResponse::Null(remote_id) => {
                        bail!("Remote data for canonical id '{remote_id}' is null")
                    }
                }
            }
            DatabaseEntry::Missing(missing) => {
                missing.commit()?;
                bail!("Citation key not present in database: '{citation_key}'");
            }
        },
        Command::Util { util_command } => match util_command {
            UtilCommand::Check => {
                info!("Validating record binary data and consistency, and checking for dangling records.");
                record_db.validate()?;
            }
            UtilCommand::List { canonical } => {
                record_db.map_citation_keys(canonical, |key_str| {
                    println!("{key_str}");
                })?;
            }
        },
    };

    Ok(())
}

fn edit_record_and_update(row: Row, record: Record) -> Result<Entry<RawRecordData>, anyhow::Error> {
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
            row.apply(row::add_alias(&alias))?;
        }

        if new_record_data != entry.data() {
            info!("Updating cached data for '{canonical}'");
            row.apply(row::update_row_data(new_record_data))?;
        }

        entry = new_entry;
    }

    row.commit()?;
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
fn validate_and_retrieve<T: Iterator<Item = RecordId>>(
    citation_keys: T,
    record_db: &mut RecordDatabase,
    client: &HttpClient,
    ignore_null: bool,
) -> BTreeMap<RemoteId, NonEmpty<Entry<RawRecordData>>> {
    let mut records: BTreeMap<RemoteId, NonEmpty<Entry<RawRecordData>>> = BTreeMap::new();

    for (bibtex_entry, canonical) in citation_keys
        .filter_map(|citation_key| {
            match get_record(record_db, citation_key, client) {
                Err(err) => {
                    error!("{err}");
                    None
                }
                Ok(GetRecordResponse::Exists(record)) => Some(record),
                Ok(GetRecordResponse::NullRemoteId(remote_id)) => {
                    if !ignore_null {
                        error!("Null record: {remote_id}");
                    }
                    None
                }
                Ok(GetRecordResponse::NullAlias(alias)) => {
                    if !ignore_null {
                        error!("Undefined alias: {alias}");
                    }
                    None
                }
            }
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
