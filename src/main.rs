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
        row::{self, RecordRow, RecordsTableRow},
        CitationKey, EntryData, RawRecordData, RecordData, RecordDatabase, RowData,
    },
    logger::Logger,
    record::{get_remote_record, GetRemoteRecordResponse, Record, RecordRowResponse},
};
pub use self::{
    entry::Entry,
    http::HttpClient,
    record::{get_record_row, Alias, RecordId, RemoteId},
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
    /// Delete records and associated keys.
    ///
    /// Delete a record, and all referencing keys (such as aliases) which are associated with the
    /// record. If there are multiple referencing keys, they will be listed so that you can confirm
    /// deletion. This can be ignored with the `--force` option.
    ///
    /// To delete an alias without deleting the underlying data, use the `autobib alias delete`
    /// command.
    Delete {
        /// The citation key to delete.
        citation_key: RecordId,
        /// Delete without prompting.
        #[arg(short, long)]
        force: bool,
    },
    /// Edit existing records.
    ///
    /// Edit an existing record using your default $EDITOR. This will open up a BibTeX file with
    /// the contents of the record. Updating the fields or the entry type will change the underlying
    /// data, and updating the entry key will create a new alias for the record.
    Edit {
        /// The citation key to edit.
        citation_key: RecordId,
    },
    /// Search for a citation key.
    ///
    /// Open an interactive picker to search for a given citation key. In order to choose the
    /// fields against which to search, use the `--fields` option.
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
        #[arg(short, long, group = "output")]
        out: Option<PathBuf>,
        /// Retrieve records but do not output BibTeX or check the validity of citation keys.
        #[arg(long, group = "output")]
        retrieve_only: bool,
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
    ///
    /// This is essentially a call to `autobib get`, except with a custom search which attempts
    /// to find citation keys inside the provided file type. The search method depends on the file
    /// type, which is determined purely based on the extension.
    Source {
        /// The files in which to search.
        paths: Vec<PathBuf>,
        /// Override file type detection.
        #[arg(long)]
        file_type: Option<SourceFileType>,
        /// Write output to file.
        #[arg(short, long, group = "output")]
        out: Option<PathBuf>,
        /// Retrieve records but do not output BibTeX or check the validity of citation keys.
        #[arg(long, group = "output")]
        retrieve_only: bool,
        /// Ignore null records and aliases.
        #[arg(long)]
        ignore_null: bool,
    },
    /// Update data associated with an existing citation key.
    ///
    /// This adds any new fields that do not exist in the current data, but will not overwrite any
    /// existing fields.
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
        /// Only list the canonical keys.
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

    // check if there was a non-fatal error during execution
    if Logger::has_error() {
        exit(1)
    }
}

/// Run the CLI.
fn run_cli(cli: Cli) -> Result<()> {
    info!("SQLite version: {}", rusqlite::version());
    info!("Autobib version: {}", env!("CARGO_PKG_VERSION"));
    info!(
        "Database binary data version: {}",
        db::binary_format_version()
    );
    info!("Database schema version: {}", db::schema_version());

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
                match get_record_row(&mut record_db, target, &client)? {
                    RecordRowResponse::Exists(_, row) => {
                        if !row.apply(row::add_alias(&alias))? {
                            error!("Alias already exists: '{alias}'");
                        }
                        row.commit()?;
                    }
                    RecordRowResponse::NullRemoteId(remote_id, missing) => {
                        missing.commit()?;
                        error!("Cannot create alias for null record '{remote_id}'");
                    }
                    RecordRowResponse::NullAlias(alias, missing) => {
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
            match record_db.initialize_row(&citation_key)? {
                RecordsTableRow::Exists(row) => {
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
                                row.commit()?;
                                bail!("Aborted deletion");
                            }
                        }
                    }

                    row.delete()?.commit()?;
                }
                RecordsTableRow::Missing(missing) => {
                    missing.commit()?;
                    bail!("Citation key '{citation_key}' does not exist.");
                }
            }
        }
        Command::Edit { citation_key } => {
            match get_record_row(&mut record_db, citation_key, &client)? {
                record::RecordRowResponse::Exists(record, row) => {
                    edit_record_and_update(row, record)?;
                }
                record::RecordRowResponse::NullRemoteId(remote_id, missing) => {
                    missing.commit()?;
                    error!("Cannot edit null record '{remote_id}'");
                }
                record::RecordRowResponse::NullAlias(alias, missing) => {
                    missing.commit()?;
                    error!("Cannot edit undefined alias '{alias}'");
                }
            }
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
        Command::Get {
            mut citation_keys,
            out,
            retrieve_only,
            ignore_null,
        } => {
            // Collect all entries which are not null
            let valid_entries = retrieve_and_validate_entries(
                citation_keys.drain(..),
                &mut record_db,
                &client,
                retrieve_only,
                ignore_null,
            );

            if !retrieve_only {
                output_entries(out.as_ref(), valid_entries)?;
            }
        }
        Command::Info {
            citation_key,
            report,
        } => match record_db.initialize_row(&citation_key)? {
            RecordsTableRow::Exists(row) => {
                match report {
                    InfoReportType::All => {
                        let row_data = row.apply(row::get_row_data)?;
                        println!("Canonical: {}", row_data.canonical);
                        println!(
                            "Equivalent references: {}",
                            row.apply(row::get_referencing_keys)?.iter().join(", ")
                        );
                        println!(
                            "Valid BibTeX? {}",
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
                            error!("Invalid BibTeX: {}", citation_key.name());
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
                };
                row.commit()?;
            }
            RecordsTableRow::Missing(missing) => {
                missing.commit()?;
                error!("Citation key '{citation_key}' does not exist.");
            }
        },
        Command::Local { id, edit, from } => {
            let remote_id = RemoteId::local(&id);

            let (row, data) = match record_db.initialize_row(&remote_id)? {
                RecordsTableRow::Exists(row) => {
                    if from.is_some() {
                        row.commit()?;
                        bail!("Local record '{id}' already exists")
                    } else {
                        let raw_record_data = row.apply(row::get_row_data)?.data;
                        (row, raw_record_data)
                    }
                }
                RecordsTableRow::Missing(missing) => {
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
        Command::Source {
            paths,
            file_type,
            out,
            retrieve_only,
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

            let valid_entries = retrieve_and_validate_entries(
                container.drain(),
                &mut record_db,
                &client,
                retrieve_only,
                ignore_null,
            );

            if !retrieve_only {
                output_entries(out.as_ref(), valid_entries)?;
            }
        }
        Command::Update { citation_key } => match record_db.initialize_row(&citation_key)? {
            RecordsTableRow::Exists(row) => {
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
                        row.commit()?;
                        error!("Remote data for canonical id '{remote_id}' is null");
                    }
                }
            }
            RecordsTableRow::Missing(missing) => {
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

fn edit_record_and_update(
    row: RecordRow,
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
fn output_entries<D: EntryData, P: AsRef<Path>>(
    out: Option<P>,
    grouped_entries: BTreeMap<RemoteId, NonEmpty<Entry<D>>>,
) -> Result<(), serde_bibtex::Error> {
    if let Some(path) = out {
        let writer = io::BufWriter::new(std::fs::File::create(path)?);
        write_entries(writer, grouped_entries)?;
    } else {
        let stdout = io::stdout();
        if stdout.is_tty() {
            // do not write an extra newline if interactive
            if !grouped_entries.is_empty() {
                write_entries(stdout, grouped_entries)?;
            }
        } else {
            let writer = io::BufWriter::new(stdout);
            write_entries(writer, grouped_entries)?;
        }
    };

    Ok(())
}

/// Iterate over records, writing the entries and warning about duplicates.
fn write_entries<W: io::Write, D: EntryData>(
    writer: W,
    grouped_entries: BTreeMap<RemoteId, NonEmpty<Entry<D>>>,
) -> Result<(), serde_bibtex::Error> {
    let mut serializer = Serializer::unchecked(writer);

    serializer.collect_seq(grouped_entries.iter().flat_map(|(canonical, entry_group)| {
        if entry_group.len() > 1 {
            warn!(
                "Multiple keys for '{canonical}': {}",
                entry_group.iter().map(Entry::key).join(", ")
            );
        };
        entry_group
    }))
}

/// Retrieve and validate BibTeX entries.
fn retrieve_and_validate_entries<T: Iterator<Item = RecordId>>(
    citation_keys: T,
    record_db: &mut RecordDatabase,
    client: &HttpClient,
    retrieve_only: bool,
    ignore_null: bool,
) -> BTreeMap<RemoteId, NonEmpty<Entry<RawRecordData>>> {
    let valid_entries = citation_keys.filter_map(|citation_key| {
        retrieve_and_validate_single_entry(
            record_db,
            citation_key,
            client,
            retrieve_only,
            ignore_null,
        )
        .unwrap_or_else(|error| {
            error!("{error}");
            None
        })
    });

    let mut grouped_entries: BTreeMap<RemoteId, NonEmpty<Entry<RawRecordData>>> = BTreeMap::new();
    for (bibtex_entry, canonical) in valid_entries {
        match grouped_entries.entry(canonical) {
            Occupied(e) => e.into_mut().push(bibtex_entry),
            Vacant(e) => {
                e.insert(NonEmpty::singleton(bibtex_entry));
            }
        }
    }
    grouped_entries
}

/// Retrieve and validate a single BibTeX entry.
fn retrieve_and_validate_single_entry(
    record_db: &mut RecordDatabase,
    citation_key: RecordId,
    client: &HttpClient,
    retrieve_only: bool,
    ignore_null: bool,
) -> Result<Option<(Entry<RawRecordData>, RemoteId)>, error::Error> {
    match get_record_row(record_db, citation_key, client)? {
        RecordRowResponse::Exists(record, row) => {
            if retrieve_only {
                row.commit()?;
                Ok(None)
            } else {
                let Record {
                    key,
                    data,
                    canonical,
                } = record;
                let entry = validate_bibtex_entry(key, data, &row).map(|entry| (entry, canonical));
                row.commit()?;
                Ok(entry)
            }
        }
        RecordRowResponse::NullRemoteId(remote_id, missing) => {
            if !ignore_null {
                error!("Null record: {remote_id}");
            }
            missing.commit()?;
            Ok(None)
        }
        RecordRowResponse::NullAlias(alias, missing) => {
            if !ignore_null {
                error!("Undefined alias: {alias}");
            }
            missing.commit()?;
            Ok(None)
        }
    }
}

/// Validate a BibTeX key and return the entry, logging errors and suggesting fixes.
fn validate_bibtex_entry(
    key: String,
    data: RawRecordData,
    row: &RecordRow,
) -> Option<Entry<RawRecordData>> {
    match Entry::try_new(key, data) {
        Ok(entry) => Some(entry),
        Err(error) => {
            if let error::BibTeXError::InvalidKey(_) = &error {
                match get_valid_referencing_keys(row) {
                    Ok(alternative_keys) => {
                        if !alternative_keys.is_empty() {
                            error!(
                                    "{error}\n  Suggested fix: use one of the following equivalent keys: {}",
                                    alternative_keys.join(", ")
                                );
                        } else {
                            error!("{error}\n  Suggested fix: create an alias which does not contain disallowed characters: {{}}(),=\\#%\"");
                        }
                    }
                    Err(error2) => {
                        error!(
                            "{error}\n  Another error occurred while retrieving equivalent keys:"
                        );
                        error!("{error2}");
                    }
                }
            } else {
                error!("{error}");
            }
            None
        }
    }
}

/// Get keys equivalent to a given key that are valid BibTeX citation keys.
fn get_valid_referencing_keys(row: &RecordRow) -> Result<Vec<String>, rusqlite::Error> {
    let mut referencing_keys = row.apply(row::get_referencing_keys)?;
    referencing_keys.retain(|k| is_entry_key(k));
    Ok(referencing_keys)
}
