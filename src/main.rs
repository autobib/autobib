pub mod cite_search;
mod config;
pub mod db;
mod entry;
pub mod error;
mod http;
mod logger;
mod normalize;
pub mod provider;
mod record;
pub mod term;

use std::{
    collections::{
        btree_map::Entry::{Occupied, Vacant},
        BTreeMap, HashMap, HashSet,
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
use serde_bibtex::{
    ser::Serializer,
    token::{is_entry_key, EntryKey},
};
use term::{Confirm, Editor, EditorConfig};

use self::{
    cite_search::{get_citekeys, SourceFileType},
    db::{
        state::{NullRecordRow, RecordIdState, RecordRow, RemoteIdState, RowData, State},
        CitationKey, EntryData, RawRecordData, RecordData, RecordDatabase,
    },
    error::{AliasConversionError, ShortError},
    logger::{suggest, Logger},
    record::{get_remote_response_recursive, Record, RecordRowResponse, RecursiveRemoteResponse},
};
pub use self::{
    config::Config,
    entry::Entry,
    http::HttpClient,
    normalize::Normalize,
    record::{get_record_row, Alias, AliasOrRemoteId, RecordId, RemoteId},
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Use record database.
    #[arg(short, long, value_name = "PATH", env = "AUTOBIB_DATABASE_PATH")]
    database: Option<PathBuf>,

    /// Use configuration file.
    #[arg(short, long, value_name = "PATH", env = "AUTOBIB_CONFIG_PATH")]
    config: Option<PathBuf>,

    /// Do not require user action.
    #[arg(short = 'I', long, global = true)]
    no_interactive: bool,

    #[command(flatten)]
    verbose: Verbosity<WarnLevel>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Copy, Clone, ValueEnum, Default)]
enum InfoReportType {
    /// Show all info.
    #[default]
    All,
    /// Print the canonical identifer.
    Canonical,
    /// Check if the key is valid bibtex.
    Valid,
    /// Print equivalent identifiers.
    Equivalent,
    /// Print the last modified time.
    Modified,
}

#[derive(Debug, Copy, Clone)]
enum UpdateMode {
    PreferCurrent,
    PreferIncoming,
    Prompt,
}

impl UpdateMode {
    fn from_flags(no_interactive: bool, prefer_current: bool, prefer_incoming: bool) -> Self {
        if prefer_incoming {
            UpdateMode::PreferIncoming
        } else if prefer_current || no_interactive {
            UpdateMode::PreferCurrent
        } else {
            UpdateMode::Prompt
        }
    }
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
        /// The citation keys to delete.
        citation_keys: Vec<RecordId>,
        /// Delete without prompting.
        #[arg(short, long)]
        force: bool,
        /// Also delete null records from the null record cache.
        #[arg(short, long)]
        delete_null: bool,
    },
    /// Edit existing records.
    ///
    /// Edit an existing record using your $EDITOR. This will open a BibTeX file with the
    /// contents of the record. Updating the fields or the entry type will change the underlying
    /// data, and updating the entry key will create a new alias for the record.
    ///
    /// Some non-interactive edit methods are supported. These can be used along with the
    /// `--no-interactive` flag to modify records without opening your $EDITOR:
    ///
    /// `--normalize-whitespace` converts whitespace blocks into a single ASCII space.
    ///
    /// `--set-eprint` accepts a list of field keys, and sets the "eprint" and
    ///   "eprinttype" bibtex fields from the first field key which is present in the record.
    Edit {
        /// The citation key to edit.
        citation_key: RecordId,
        /// Normalize whitespace.
        #[arg(long)]
        normalize_whitespace: bool,
        /// Set "eprint" and "eprinttype" BibTeX fields from provided fields.
        #[arg(long, value_delimiter = ',')]
        set_eprint: Vec<String>,
    },
    /// Search for a citation key.
    ///
    /// Open an interactive picker to search for a given citation key. In order to choose the
    /// fields against which to search, use the `--fields` option.
    Find {
        /// Fields to search (e.g. author, title).
        #[arg(short, long, value_delimiter = ',')]
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
        #[arg(short, long, value_enum, default_value_t)]
        report: InfoReportType,
    },
    /// Create or edit a local record with the given handle.
    Local {
        /// The name for the record.
        id: String,
        /// Create local record from bibtex file.
        #[arg(short, long, value_name = "PATH", group = "input")]
        from: Option<PathBuf>,
        /// Rename an existing local record.
        #[arg(long, value_name = "EXISTING_ID", group = "input")]
        rename_from: Option<String>,
        /// Do not create the alias `<ID>` for `local:<ID>`.
        #[arg(long)]
        no_alias: bool,
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
    /// By default, you will be prompted if there is a conflict between the current and incoming
    /// records.
    ///
    /// To override this behaviour, use the `--prefer-current` or `--prefer-incoming`
    /// flag; `--prefer-incoming` takes precedence over `--prefer-current`.
    /// The `--no-interactive` global flag implies `--prefer-current`.
    Update {
        /// The citation key to update.
        citation_key: RecordId,
        /// Read update data from local path.
        #[arg(short, long, value_name = "PATH")]
        from: Option<PathBuf>,
        /// Keep the current value without prompting in the event of a conflict.
        #[arg(long, group = "update-mode")]
        prefer_current: bool,
        /// Update with the incoming value without prompting in the event of a conflict.
        #[arg(long, group = "update-mode")]
        prefer_incoming: bool,
    },
    /// Utilities to manage database.
    Util {
        #[command(subcommand)]
        util_command: UtilCommand,
    },
}

/// Parse an instance of type `T` using its [`FromStr`] implementation, but instead use the
/// [`ShortError`] implementation of the error instead of the usual error message.
///
/// This is particularly useful for command line error messages, where some information is already
/// displayed automatically by clap.
fn with_short_err<T: FromStr>(input: &str) -> Result<T, &'static str>
where
    <T as FromStr>::Err: ShortError,
{
    T::from_str(input).map_err(|err| err.short_err())
}

/// Manage aliases.
#[derive(Subcommand)]
enum AliasCommand {
    /// Add a new alias.
    Add {
        /// The new alias to create.
        #[arg(value_parser = with_short_err::<Alias>)]
        alias: Alias,
        /// What the alias points to.
        target: RecordId,
    },
    /// Delete an existing alias.
    #[command(alias = "rm")]
    Delete {
        /// The existing alias to delete.
        #[arg(value_parser = with_short_err::<Alias>)]
        alias: Alias,
    },
    /// Rename an existing alias.
    #[command(alias = "mv")]
    Rename {
        /// The name of the existing alias.
        #[arg(value_parser = with_short_err::<Alias>)]
        alias: Alias,
        /// The name of the new alias.
        new: Alias,
    },
}

/// Manage aliases.
#[derive(Subcommand)]
enum UtilCommand {
    /// Check database for errors.
    Check {
        /// Attempt to fix errors, printing any errors which could not be fixed.
        #[arg(short, long)]
        fix: bool,
    },
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

    let strategy = choose_app_strategy(AppStrategyArgs {
        top_level_domain: "org".to_owned(),
        author: env!("CARGO_PKG_NAME").to_owned(),
        app_name: env!("CARGO_PKG_NAME").to_owned(),
    })?;

    // Open or create the database
    let mut record_db = if let Some(db_path) = cli.database {
        // at a user-provided path
        info!("Using user-provided database file '{}'", db_path.display());

        RecordDatabase::open(db_path)?
    } else {
        // at the default path
        let data_dir = strategy.data_dir();

        create_dir_all(&data_dir)?;
        let default_db_path = data_dir.join("records.db");
        info!(
            "Using default database file '{}'",
            default_db_path.display()
        );

        RecordDatabase::open(default_db_path)?
    };

    // Read configuration from filesystem
    let config = if let Some(config_path) = cli.config {
        Config::load(config_path, false)?
    } else {
        Config::load(strategy.config_dir().join("config.toml"), true)?
    };

    // Initialize the reqwest Client
    let builder = HttpClient::default_builder();
    let client = HttpClient::new(builder)?;

    // Run the cli
    match cli.command {
        Command::Alias { alias_command } => match alias_command {
            AliasCommand::Add { alias, target } => {
                info!("Creating alias '{alias}' for '{target}'");
                let (_, row) = get_record_row(&mut record_db, target, &client, &config.on_insert)?
                    .exists_or_commit_null("Cannot create alias for")?;
                if !row.add_alias(&alias)? {
                    error!("Alias already exists: '{alias}'");
                }
                row.commit()?;
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
            citation_keys,
            force,
            delete_null,
        } => {
            let deduplicated = filter_and_deduplicate_by_canonical(
                citation_keys.into_iter(),
                &mut record_db,
                |remote_id, null_row| {
                    if !delete_null {
                        null_row.commit()?;
                        error!("Null record found for '{remote_id}'");
                        suggest!("Use the `--delete-null` option to also delete null records.");
                    } else {
                        null_row.delete()?.commit()?;
                    }
                    Ok(())
                },
            )?;

            for (canonical, to_delete) in deduplicated {
                if let Some(row) = record_db.state_from_remote_id(&canonical)?.exists() {
                    if !force {
                        let mut unreferenced = row
                            .get_referencing_keys()?
                            .into_iter()
                            .filter(|key| !to_delete.contains(key))
                            .peekable();

                        // there are associated keys which are not present in the deletion list
                        if unreferenced.peek().is_some() {
                            if cli.no_interactive {
                                // non-interactive: skip the key
                                row.commit()?;
                                error!("Record with canonical identifier '{canonical}' has associated keys which are not requested for deletion: {}",
                                    unreferenced.join(", "));
                                suggest!("Re-run with `--force` to delete anyway.");
                                continue;
                            } else {
                                // interactive: prompt for deletion
                                eprintln!("Deleting record with canonical identifier '{canonical}' will also delete associated keys:");
                                for key in unreferenced {
                                    eprintln!("  {key}");
                                }
                                let prompt = Confirm::new("Delete anyway?", false);
                                if !prompt.confirm()? {
                                    row.commit()?;
                                    error!(
                                        "Aborted deletion of '{canonical}' via keys: '{}'",
                                        to_delete.iter().join(", ")
                                    );
                                    continue;
                                }
                            }
                        }
                    }
                    row.save_to_changelog()?;
                    row.delete()?.commit()?;
                } else {
                    error!("Database changed during deletion operation! Record {canonical} is no longer present in the database.");
                }
            }
        }
        Command::Edit {
            citation_key,
            normalize_whitespace,
            set_eprint,
        } => {
            let (mut record, row) =
                get_record_row(&mut record_db, citation_key, &client, &config.on_insert)?
                    .exists_or_commit_null("Cannot edit")?;

            if normalize_whitespace || !set_eprint.is_empty() {
                let mut data: RecordData = (&record.data).into();
                if normalize_whitespace {
                    data.normalize_whitespace();
                }
                if !set_eprint.is_empty() {
                    data.set_eprint(set_eprint.iter());
                }

                let new_data = (&data).into();
                row.save_to_changelog()?;
                row.update_row_data(&new_data)?;

                record.data = new_data;
            }

            if !cli.no_interactive {
                edit_record_and_update(&row, record)?;
            }
            row.commit()?;
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
            citation_keys,
            out,
            retrieve_only,
            ignore_null,
        } => {
            // Collect all entries which are not null
            let valid_entries = retrieve_and_validate_entries(
                citation_keys.into_iter(),
                &mut record_db,
                &client,
                retrieve_only,
                ignore_null,
                &config,
            );

            if !retrieve_only {
                output_entries(out.as_ref(), valid_entries)?;
            }
        }
        Command::Info {
            citation_key,
            report,
        } => match record_db.state_from_record_id(citation_key)? {
            RecordIdState::Existent(record_id, row) => {
                match report {
                    InfoReportType::All => {
                        let row_data = row.get_data()?;
                        println!("Canonical: {}", row_data.canonical);
                        println!(
                            "Equivalent references: {}",
                            row.get_referencing_keys()?.iter().join(", ")
                        );
                        println!(
                            "Valid BibTeX? {}",
                            if is_entry_key(record_id.name()) {
                                "yes"
                            } else {
                                "no"
                            }
                        );
                        println!("Data last modified: {}", row_data.modified);
                    }
                    InfoReportType::Canonical => {
                        println!("{}", row.get_canonical()?);
                    }

                    InfoReportType::Valid => {
                        if !is_entry_key(record_id.name()) {
                            error!("Invalid BibTeX: {}", record_id.name());
                        }
                    }
                    InfoReportType::Equivalent => {
                        for re in row.get_referencing_keys()? {
                            println!("{re}");
                        }
                    }
                    InfoReportType::Modified => {
                        println!("{}", row.last_modified()?);
                    }
                };
                row.commit()?;
            }
            RecordIdState::NullRemoteId(remote_id, null_row) => match report {
                InfoReportType::All => {
                    println!("Null record: {remote_id}");
                    let null_row_data = null_row.get_data()?;
                    println!("Last attempted: {}", null_row_data.attempted);
                }
                InfoReportType::Canonical => {
                    bail!("No canonical id for null record '{remote_id}'");
                }
                InfoReportType::Valid => {
                    bail!("Null record '{remote_id}' is automatically invalid");
                }
                InfoReportType::Equivalent => {
                    bail!("No equivalent keys for null record '{remote_id}'");
                }
                InfoReportType::Modified => {
                    println!("{}", null_row.get_null_attempted()?);
                }
            },
            RecordIdState::UnknownRemoteId(remote_id, missing) => {
                missing.commit()?;
                bail!("Cannot obtain report for record not in database: '{remote_id}'");
            }
            RecordIdState::UndefinedAlias(alias) => {
                bail!("Cannot obtain report for undefined alias: '{alias}'");
            }
            RecordIdState::InvalidRemoteId(err) => bail!("{err}"),
        },
        Command::Local {
            id,
            from,
            rename_from,
            no_alias,
        } => {
            let alias = match Alias::from_str(&id) {
                Ok(alias) => alias,
                Err(AliasConversionError::Empty(_)) => {
                    bail!("local sub-id must contain non-whitespace characters");
                }
                Err(AliasConversionError::IsRemoteId(_)) => {
                    bail!("local sub-id must not contain a colon");
                }
            };
            let remote_id = RemoteId::local(&alias);

            let (row, data) = if let Some(old_id) = rename_from {
                // Allowing for arbitrary `old_id` without validation and trimming
                // so that local ids that were valid in an older version can be renamed.
                // SAFETY: This is safe as a colon is present in the `full_id`.
                let old_remote_id =
                    unsafe { RemoteId::from_string_unchecked("local:".to_owned() + &old_id) };
                match record_db.state_from_remote_id(&old_remote_id)? {
                    RemoteIdState::Existent(row) => {
                        if !row.change_canonical_id(&remote_id)? {
                            bail!("Local record '{remote_id}' already exists")
                        }

                        if let Ok(old_alias) = Alias::from_str(&old_id) {
                            row.delete_alias_if_associated(&old_alias)?;
                        }

                        let raw_record_data = row.get_data()?.data;
                        (row, raw_record_data)
                    }
                    RemoteIdState::Null(null_row) => {
                        null_row.commit()?;
                        error!("'{remote_id}' was found in the 'NullRecords' table. A local record should not be present in the 'NullRecords' table.");
                        suggest!("Run `autobib delete --delete-null '{remote_id}'` to remove the null record.");
                        return Ok(());
                    }
                    RemoteIdState::Unknown(missing) => {
                        missing.commit()?;
                        bail!("Local record '{old_remote_id}' does not exist");
                    }
                }
            } else {
                match record_db.state_from_remote_id(&remote_id)? {
                    RemoteIdState::Existent(row) => {
                        if from.is_some() {
                            row.commit()?;
                            bail!("Local record '{remote_id}' already exists")
                        } else {
                            let raw_record_data = row.get_data()?.data;
                            (row, raw_record_data)
                        }
                    }
                    RemoteIdState::Unknown(missing) => {
                        let data = data_from_path_or_default(from.as_ref())?;
                        let row = missing.insert_and_ref(&data, &remote_id)?;
                        (row, data)
                    }
                    RemoteIdState::Null(null_row) => {
                        null_row.commit()?;
                        error!("'{remote_id}' was found in the 'NullRecords' table. A local record should not be present in the 'NullRecords' table.");
                        suggest!("Run `autobib delete --delete-null '{remote_id}'` to remove the null record.");
                        return Ok(());
                    }
                }
            };

            if !no_alias {
                info!("Creating alias '{alias}' for '{remote_id}'");
                if let Some(other_remote_id) = row.ensure_alias(&alias)? {
                    warn!("Alias '{alias}' already exists and refers to '{other_remote_id}'. '{remote_id}' will be a different record.");
                }
            }

            if !cli.no_interactive {
                edit_record_and_update(
                    &row,
                    Record {
                        key: remote_id.to_string(),
                        data,
                        canonical: remote_id,
                    },
                )?;
            }
            row.commit()?;
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
                container.into_iter(),
                &mut record_db,
                &client,
                retrieve_only,
                ignore_null,
                &config,
            );

            if !retrieve_only {
                output_entries(out.as_ref(), valid_entries)?;
            }
        }
        Command::Update {
            citation_key,
            from,
            prefer_current,
            prefer_incoming,
        } => match record_db.state_from_record_id(citation_key)? {
            RecordIdState::Existent(citation_key, row) => {
                let RowData {
                    data: existing_raw_data,
                    canonical,
                    ..
                } = row.get_data()?;
                let new_raw_data = match data_from_path_or_remote(from, canonical, &client) {
                    Ok((data, _)) => data,
                    Err(err) => {
                        row.commit()?;
                        bail!(err);
                    }
                };
                match UpdateMode::from_flags(cli.no_interactive, prefer_current, prefer_incoming) {
                    UpdateMode::PreferCurrent => {
                        info!("Updating '{citation_key}' with new data, skipping existing fields");
                        let mut existing_record = RecordData::from(&existing_raw_data);
                        existing_record.merge_or_skip(new_raw_data)?;
                        row.save_to_changelog()?;
                        row.update_row_data(&(&existing_record).into())?;
                        row.commit()?;
                    }
                    UpdateMode::PreferIncoming => {
                        info!(
                            "Updating '{citation_key}' with new data, overwriting existing fields"
                        );
                        let mut new_record = RecordData::from(&new_raw_data);
                        new_record.merge_or_skip(existing_raw_data)?;
                        row.save_to_changelog()?;
                        row.update_row_data(&(&new_record).into())?;
                        row.commit()?;
                    }
                    UpdateMode::Prompt => {
                        info!("Updating '{citation_key}' with new data");
                        let mut existing_record = RecordData::from(&existing_raw_data);
                        existing_record.merge_with_callback(
                            new_raw_data,
                            |key, current, incoming| {
                                eprintln!("Conflict for the field '{key}':");
                                eprintln!("   Current value: {current}");
                                eprintln!("  Incoming value: {incoming}");
                                let prompt = Confirm::new("Accept incoming value?", false);
                                match prompt.confirm() {
                                    Ok(true) => incoming.to_owned(),
                                    Ok(false) => current.to_owned(),
                                    Err(error) => {
                                        error!("{error}");
                                        warn!("Keeping current value for '{key}'");
                                        current.to_owned()
                                    }
                                }
                            },
                        )?;
                        row.save_to_changelog()?;
                        row.update_row_data(&(&existing_record).into())?;
                        row.commit()?;
                    }
                }
            }
            RecordIdState::NullRemoteId(remote_id, null_row) => {
                match data_from_path_or_remote(from, remote_id, &client) {
                    Ok((data, canonical)) => {
                        info!("Existing row was null; inserting new data.");
                        let row = null_row.delete()?.insert(&data, &canonical)?;
                        row.commit()?;
                    }
                    Err(err) => {
                        null_row.commit()?;
                        bail!(err);
                    }
                };
            }
            RecordIdState::UnknownRemoteId(remote_id, missing) => {
                error!("Record corresponding to '{remote_id}' does not exist in database");
                if !remote_id.is_local() {
                    suggest!("Use `autobib get` to retrieve record");
                }
                missing.commit()?;
            }
            RecordIdState::UndefinedAlias(alias) => {
                bail!("Undefined alias: '{alias}'");
            }
            RecordIdState::InvalidRemoteId(err) => bail!("{err}"),
        },
        Command::Util { util_command } => match util_command {
            UtilCommand::Check { fix } => {
                info!("Validating record binary data and consistency, and checking for dangling records.");
                let faults = record_db.recover(fix)?;
                if !faults.is_empty() {
                    error!("Erroneous data found in the database.");
                    for fault in faults {
                        eprintln!("DATABASE ERROR: {fault}");
                    }
                }
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

/// Lookup citation keys from the database, filtering out unknown and invalid remote ids and
/// undefined aliases.
///
/// Null identifiers are filtered using the provided `null_callback`.
///
/// The resulting hash map has keys which are the set of all unique canonical identifiers
/// corresponding to those citation keys which were present in the database, and values which are
/// the corresponding referencing citation keys which were initially present in the list.
fn filter_and_deduplicate_by_canonical<T, N>(
    citation_keys: T,
    record_db: &mut RecordDatabase,
    mut null_callback: N,
) -> Result<HashMap<RemoteId, HashSet<String>>, rusqlite::Error>
where
    T: Iterator<Item = RecordId>,
    N: FnMut(RemoteId, State<NullRecordRow>) -> Result<(), rusqlite::Error>,
{
    let mut deduplicated = HashMap::new();

    for record_id in citation_keys {
        match record_db.state_from_record_id(record_id)? {
            RecordIdState::Existent(remote_id, row) => {
                deduplicated
                    .entry(row.get_canonical()?)
                    .or_insert_with(HashSet::new)
                    .insert(remote_id.into());
                row.commit()?;
            }
            RecordIdState::NullRemoteId(remote_id, null_row) => {
                null_callback(remote_id, null_row)?;
            }
            RecordIdState::UnknownRemoteId(remote_id, missing) => {
                missing.commit()?;
                error!("Identifier not in database: '{remote_id}'");
            }
            RecordIdState::UndefinedAlias(alias) => {
                error!("Undefined alias: '{alias}'");
            }
            RecordIdState::InvalidRemoteId(err) => error!("{err}"),
        }
    }
    Ok(deduplicated)
}

/// Either obtain data from a `.bib` file at the provided path, or look up data from the
/// provider.
fn data_from_path_or_remote<P: AsRef<Path>>(
    maybe_path: Option<P>,
    remote_id: RemoteId,
    client: &HttpClient,
) -> Result<(RawRecordData, RemoteId), anyhow::Error> {
    if let Some(path) = maybe_path {
        Ok((data_from_path(path)?, remote_id))
    } else {
        match get_remote_response_recursive(remote_id, client)? {
            RecursiveRemoteResponse::Exists(record_data, canonical) => {
                Ok((RawRecordData::from(&record_data), canonical))
            }
            RecursiveRemoteResponse::Null(null_remote_id) => {
                bail!("Remote data for canonical id '{null_remote_id}' is null");
            }
        }
    }
}

/// Either obtain data from a `.bib` file at the provided path, or return the default data.
fn data_from_path_or_default<P: AsRef<Path>>(
    maybe_path: Option<P>,
) -> Result<RawRecordData, anyhow::Error> {
    if let Some(path) = maybe_path {
        data_from_path(path)
    } else {
        Ok((&RecordData::default()).into())
    }
}

/// Obtain data from a bibtex record at a provided path.
fn data_from_path<P: AsRef<Path>>(path: P) -> Result<RawRecordData, anyhow::Error> {
    let bibtex = read_to_string(path)?;
    let entry = Entry::<RawRecordData>::from_str(&bibtex)?;
    Ok(entry.record_data)
}

/// Edit a record and update the entry corresponding to the [`RecordRow`].
fn edit_record_and_update(
    row: &State<RecordRow>,
    record: Record,
) -> Result<Entry<RawRecordData>, anyhow::Error> {
    let Record {
        key,
        data,
        canonical,
    } = record;

    let mut entry = Entry::new(EntryKey::new(key).map_err(|res| res.error)?, data);

    let editor = Editor::new(EditorConfig { suffix: ".bib" });

    if let Some(new_entry) = editor.edit(&entry)? {
        let Entry {
            key: ref new_key,
            record_data: ref new_record_data,
        } = new_entry;

        if new_key != entry.key() {
            let alias = Alias::from_str(new_key.as_ref())?;
            info!("Creating new alias '{alias}' for '{canonical}'");
            row.add_alias(&alias)?;
        }

        if new_record_data != entry.data() {
            info!("Updating cached data for '{canonical}'");
            row.save_to_changelog()?;
            row.update_row_data(new_record_data)?;
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
                entry_group.iter().map(|e| e.key().as_ref()).join(", ")
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
    config: &Config,
) -> BTreeMap<RemoteId, NonEmpty<Entry<RawRecordData>>> {
    let valid_entries = citation_keys.filter_map(|citation_key| {
        retrieve_and_validate_single_entry(
            record_db,
            citation_key,
            client,
            retrieve_only,
            ignore_null,
            config,
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
    config: &Config,
) -> Result<Option<(Entry<RawRecordData>, RemoteId)>, error::Error> {
    match get_record_row(record_db, citation_key, client, &config.on_insert)? {
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
                let entry =
                    validate_bibtex_key(key, &row).map(|key| (Entry::new(key, data), canonical));
                row.commit()?;
                Ok(entry)
            }
        }
        RecordRowResponse::NullRemoteId(remote_id, missing) => {
            if !ignore_null {
                error!("Null record: '{remote_id}'");
            }
            missing.commit()?;
            Ok(None)
        }
        RecordRowResponse::NullAlias(alias) => {
            if !ignore_null {
                error!("Undefined alias: '{alias}'");
            }
            Ok(None)
        }
        RecordRowResponse::InvalidRemoteId(err) => {
            error!("{err}");
            Ok(None)
        }
    }
}

/// Validate a BibTeX key, logging errors and suggesting fixes.
fn validate_bibtex_key(key: String, row: &State<RecordRow>) -> Option<EntryKey<String>> {
    match EntryKey::new(key) {
        Ok(bibtex_key) => Some(bibtex_key),
        Err(parse_result) => {
            match get_valid_referencing_keys(row) {
                Ok(alternative_keys) => {
                    if !alternative_keys.is_empty() {
                        error!("{}", parse_result.error,);
                        suggest!(
                            "Use one of the following equivalent keys: {}",
                            alternative_keys.join(", ")
                        );
                    } else {
                        error!("{}", parse_result.error);
                        suggest!("Create an alias which does not contain whitespace or disallowed characters: {{}}(),=\\#%\"");
                    }
                }
                Err(error2) => {
                    error!(
                        "{}\n  Another error occurred while retrieving equivalent keys:",
                        parse_result.error
                    );
                    error!("{error2}");
                }
            }
            None
        }
    }
}

/// Get keys equivalent to a given key that are valid BibTeX citation keys.
fn get_valid_referencing_keys(row: &State<RecordRow>) -> Result<Vec<String>, rusqlite::Error> {
    let mut referencing_keys = row.get_referencing_keys()?;
    referencing_keys.retain(|k| is_entry_key(k));
    Ok(referencing_keys)
}
