use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    str::FromStr,
};

use anyhow::Result;
use chrono::{DateTime, Local};
use clap::{
    CommandFactory, Parser, Subcommand, ValueEnum, builder::ArgPredicate, error::ErrorKind,
};
use clap_complete::aot::Shell;
use clap_verbosity_flag::{Verbosity, WarnLevel};
use crossterm::style::Stylize;

use crate::{
    cite_search::SourceFileType,
    db::state::RevisionId,
    entry::{EntryType, FieldKey, SetFieldCommand},
    error::ShortError,
    format::Template,
    record::{Alias, RecordId},
};

/// Determine the default value for `no_interactive` based on interactivity of stdin and stderr.
fn determine_no_interactive() -> bool {
    !(io::stdin().is_terminal() && io::stderr().is_terminal())
}

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Use record database.
    #[arg(
        short = 'D',
        long,
        value_name = "PATH",
        env = "AUTOBIB_DATABASE_PATH",
        global = true
    )]
    pub database: Option<PathBuf>,
    /// Use configuration file.
    #[arg(
        short = 'C',
        long,
        value_name = "PATH",
        env = "AUTOBIB_CONFIG_PATH",
        global = true
    )]
    pub config: Option<PathBuf>,
    /// Use directory for attachments.
    #[arg(long, value_name = "PATH", env = "AUTOBIB_ATTACHMENTS_DIRECTORY")]
    pub attachments_dir: Option<PathBuf>,
    /// Do not require user action.
    ///
    /// This option is set automatically if the standard input is not a terminal.
    #[arg(short = 'I', long, global = true, default_value_t = determine_no_interactive())]
    pub no_interactive: bool,
    /// Open the database in read-only mode.
    #[arg(long)]
    pub read_only: bool,
    #[command(flatten)]
    pub verbose: Verbosity<WarnLevel>,
}

#[derive(Debug, Copy, Clone, ValueEnum, Default)]
pub enum InfoReportType {
    /// Show all info.
    #[default]
    #[value(alias("a"))]
    All,
    /// Print the canonical identifer.
    #[value(alias("c"))]
    Canonical,
    /// Check if the key is valid BibTeX.
    #[value(alias("v"))]
    Valid,
    /// Print equivalent identifiers.
    #[value(alias("e"))]
    Equivalent,
    /// Print the last modified time.
    #[value(alias("m"))]
    Modified,
    /// Print the revision.
    #[value(alias("m"))]
    Revision,
}

#[derive(Debug, Copy, Clone, ValueEnum)]
pub enum OnConflict {
    /// Always keep current values.
    ///
    /// This is the default if the terminal is not interactive.
    #[value(alias("c"), alias("current"))]
    PreferCurrent,
    /// Overwrite current values.
    #[value(alias("i"), alias("incoming"))]
    PreferIncoming,
    /// Prompt if the there is a conflict.
    #[value(alias("p"))]
    Prompt,
}

impl Default for OnConflict {
    fn default() -> Self {
        if determine_no_interactive() {
            Self::PreferCurrent
        } else {
            Self::Prompt
        }
    }
}

#[derive(Debug, Copy, Clone, ValueEnum, Default)]
pub enum FindMode {
    /// Search record attachments and print the selected path.
    Attachments,
    /// Search records and print the selected canonical identifier.
    #[default]
    CanonicalId,
}

#[derive(Debug, Copy, Clone, ValueEnum, Default)]
pub enum ImportMode {
    /// Import as `local:` records.
    #[default]
    #[value(alias("l"))]
    Local,
    /// Use automatically determined keys.
    #[value(alias("k"))]
    DetermineKey,
    /// Use automatically determined keys, first retrieving from remote.
    #[value(alias("r"))]
    Retrieve,
    /// Only determine the key and retrieve from remote.
    #[value(alias("R"))]
    RetrieveOnly,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage aliases.
    Alias {
        #[command(subcommand)]
        alias_command: AliasCommand,
    },
    /// Attach a file.
    ///
    /// Add a new file to the directory associated with a record, as determined by the `path`
    /// subcommand. The original file is copied to the new directory, or can be renamed
    /// with the `--rename` option.
    Attach {
        /// The record to associate the file with.
        identifier: RecordId,
        /// The path or URL for the file to add.
        file: String,
        /// Rename the file.
        #[arg(short, long)]
        rename: Option<PathBuf>,
        /// Overwrite an existing file with the same name.
        #[arg(short, long)]
        force: bool,
    },
    /// Generate a shell completions script.
    #[clap(hide = true)]
    Completions {
        /// The shell for which to generate the script.
        shell: Shell,
    },
    /// Generate configuration file.
    #[clap(hide = true)]
    DefaultConfig,
    /// Delete records and associated keys.
    ///
    /// By default, this performs a soft delete, where the current data and keys are retained
    /// but future attempts to read them will result in an error. The data can be recovered with
    /// `autobib hist undo`. The key provided with the `--replace` option will be used to suggest
    /// replacements.
    ///
    /// With the `--hard` option, the data as well as all keys are deleted permanently. This is
    /// incompatible with the `--replace` option.
    Delete {
        /// The records to delete.
        identifiers: Vec<RecordId>,
        /// A replacement key.
        #[arg(short, long, group = "delete_mode")]
        replace: Option<RecordId>,
        /// Hard deletion, which removes all history and aliases, and cannot be undone.
        #[arg(long, group = "delete_mode")]
        hard: bool,
        /// Update aliases, either deleting or changing them to point to the new row if `--replace` is specified.
        #[arg(long)]
        update_aliases: bool,
    },
    /// Edit existing records.
    ///
    /// Edit an existing record using your $EDITOR. This will open a BibTeX file with the
    /// contents of the record. Updating the fields or the entry type will change the underlying
    /// data, and updating the entry key will create a new alias for the record.
    ///
    /// Some non-interactive edit methods are also supported. If any are specified, they will
    /// modify the record without opening your $EDITOR:
    ///
    /// `--normalize-whitespace` converts whitespace blocks into a single ASCII space.
    ///
    /// `--set-eprint` accepts a list of field keys, and sets the "eprint" and
    ///   "eprinttype" BibTeX fields from the first field key which is present in the record.
    ///
    /// `--strip-journal-series` strips a trailing journal series from the `journal` field
    Edit {
        /// The record(s) to edit.
        identifiers: Vec<RecordId>,
        /// Normalize whitespace.
        #[arg(long)]
        normalize_whitespace: bool,
        /// Set "eprint" and "eprinttype" BibTeX fields from provided fields.
        #[arg(long, value_delimiter = ',', value_name = "FIELD_KEY")]
        set_eprint: Vec<String>,
        /// Strip trailing journal series
        #[arg(long)]
        strip_journal_series: bool,
        /// Set the entry type.
        #[arg(long, value_name = "ENTRY_TYPE")]
        update_entry_type: Option<EntryType>,
        /// Delete a field. This is performed before setting field values.
        #[arg(long, value_name = "FIELD_KEY")]
        delete_field: Vec<FieldKey>,
        /// Set specific field values using BibTeX field syntax
        #[arg(long, value_name = "FIELD_KEY={VALUE}")]
        set_field: Vec<SetFieldCommand>,
        /// Insert a new copy with updated modification time regardless of changes.
        #[arg(long)]
        touch: bool,
    },
    /// Search for an identifier.
    ///
    /// Open an interactive picker to search for a given identifier. The lines in the
    /// picker are rendered using the template provided by the `--format` option, falling
    /// back to the config value or a default template.
    Find {
        /// Set the format template.
        #[arg(short, long)]
        template: Option<Template>,
        /// Only include records which contain all of the fields in the template.
        #[arg(short, long)]
        strict: bool,
        /// The type of search to perform.
        #[arg(short, long, value_enum, default_value_t)]
        mode: FindMode,
    },
    /// Retrieve records given identifiers.
    Get {
        /// The identifiers to retrieve.
        identifiers: Vec<RecordId>,
        /// Write output to file.
        #[arg(short, long, group = "output", value_name = "PATH")]
        out: Option<PathBuf>,
        /// Append new entries to the output, skipping existing entries.
        #[arg(short, long, requires = "out")]
        append: bool,
        /// Retrieve records but do not output BibTeX or check the validity of identifiers as
        /// valid BibTeX keys.
        #[arg(long, group = "output")]
        retrieve_only: bool,
        /// Ignore null records and aliases.
        #[arg(long)]
        ignore_null: bool,
    },
    /// Commands to manipulate version history.
    Hist {
        #[command(subcommand)]
        hist_command: HistCommand,
    },
    /// Import records from a BibTeX file.
    ///
    /// The implementation automatically determines a remote identifier from the data, using
    /// your `preferred_providers` config setting and with unspecified fallback if there is no
    /// match.
    /// Use `--local-fallback` to import as `local:` identifiers if this process fails.
    ///
    /// With default flag values, importing is idempotent: if you run an import twice, the result
    /// will be no different than running this method once, and duplicate entries will not be
    /// created.
    ///
    /// Failed imports are printed to STDOUT with the error messages inside comments. A potential workflow is to redirect output a file, edit
    /// the file to resolve issues, and then import again.
    ///
    /// If you use the `--retrieve` option, the determined identifier can be a reference identifier,
    /// which will be converted into a canonical identifier using a remote API call.
    Import {
        /// The BibTeX file(s) from which to import.
        targets: Vec<PathBuf>,
        #[arg(short, long)]
        /// Map the citation keys to local identifiers if provenance could not be determined.
        local_fallback: bool,
        /// How to resolve conflicts with data currently present in your database.
        #[arg(
            short = 'n',
            long,
            value_enum,
            default_value_t = OnConflict::PreferCurrent,
        )]
        on_conflict: OnConflict,
        /// Never create aliases.
        #[arg(short = 'A', long)]
        no_alias: bool,
        /// Make a remote request to resolve reference providers if a canonical cannot be found.
        #[arg(long)]
        resolve: bool,
        /// Attach files specified in the `file` field.
        #[arg(long)]
        include_files: bool,
    },
    /// Show metadata associated with an identifier.
    Info {
        /// The identifier.
        identifier: RecordId,
        /// The type of information to display.
        #[arg(short, long, value_enum, default_value_t)]
        report: InfoReportType,
    },
    /// Create a local record with the given handle.
    ///
    /// If no arguments are specified, you will be prompted to edit the local record before adding it to the
    /// database. If the terminal is non-interactive or `--no-interactive` is set, this will insert
    /// a default value with no contents.
    ///
    /// You can provide BibTeX data from a file with the `--from-bibtex` option, or by providing
    /// values using `--with-entry-type` and `--with-field`.
    ///
    /// The `--with-entry-type` or `--with-field` values will override any
    /// values present in the data read from the BibTeX file.
    ///
    /// This fails if the local identifier already exists in the database.
    Local {
        /// The name for the record.
        id: String,
        /// Create the record using the provided BibTeX data.
        #[arg(short = 'b', long, value_name = "PATH", group = "input")]
        from_bibtex: Option<PathBuf>,
        /// Set the entry type.
        #[arg(long, value_name = "ENTRY_TYPE")]
        with_entry_type: Option<EntryType>,
        /// Set specific field values using BibTeX `key = {value}` syntax
        #[arg(long, value_name = "FIELD_KEY={VALUE}")]
        with_field: Vec<SetFieldCommand>,
    },
    /// Display the revision history associated with an identifier.
    Log {
        /// The identifier.
        identifier: RecordId,
        /// Show parallel changes, instead of only the history of the active version.
        #[arg(short, long)]
        tree: bool,
        /// Also traverse pass deletion markers.
        #[arg(short, long)]
        all: bool,
        /// Display newest changes first.
        #[arg(short, long)]
        reverse: bool,
    },
    /// Show attachment directory associated with record.
    Path {
        /// Show directory path associated with this identifier.
        identifier: RecordId,
        /// Also create the directory.
        #[arg(short, long)]
        mkdir: bool,
    },
    /// Generate records by searching for identifiers inside files.
    ///
    /// This is essentially a call to `autobib get`, except with a custom search which attempts
    /// to find identifiers inside the provided file(s), typically as citation keys.
    /// The search method depends on the file
    /// type, which is determined purely based on the extension.
    Source {
        /// The files in which to search.
        paths: Vec<PathBuf>,
        /// Override file type detection.
        #[arg(long, value_name = "FILETYPE")]
        file_type: Option<SourceFileType>,
        /// Write output to file.
        #[arg(short, long, group = "output", value_name = "PATH")]
        out: Option<PathBuf>,
        /// Search in standard input interpreted as the provided file type.
        #[arg(long, value_name = "FILETYPE")]
        stdin: Option<SourceFileType>,
        /// Append new entries to the output.
        #[arg(short, long, requires = "out")]
        append: bool,
        /// Retrieve records but do not output BibTeX or check the validity of identifiers.
        #[arg(long, group = "output")]
        retrieve_only: bool,
        /// Only print the identifiers keys which were found (sorted and deduplicated).
        #[arg(long, group = "output")]
        print_keys: bool,
        /// Skip an identifier (if present).
        #[arg(short, long, value_name = "IDENTIFIERS")]
        skip: Vec<RecordId>,
        /// Skip identifiers which are present in the provided file(s).
        #[arg(long, value_name = "PATH")]
        skip_from: Vec<PathBuf>,
        /// Override file type detection for skip files.
        #[arg(long, value_name = "FILETYPE")]
        skip_file_type: Option<SourceFileType>,
        /// Ignore null records and aliases.
        #[arg(long)]
        ignore_null: bool,
    },
    /// Update data associated with an identifier.
    ///
    /// By default, you will be prompted if there is a conflict between the current and incoming
    /// records.
    ///
    /// To override this behaviour, use `-n prefer-current` or `-n prefer-incoming`.
    /// If the terminal is not interactive or the `--no-interactive` global option is set, this
    /// will result in an error if the `-n prefer-current` or `-n prefer-incoming` is not explicitly set.
    Update {
        /// The identifier for the update operation.
        identifier: RecordId,
        /// Read update data from a BibTeX entry in a file.
        #[arg(short = 'b', long, value_name = "PATH", group = "update_from")]
        from_bibtex: Option<PathBuf>,
        /// Read update data from other record data.
        #[arg(short = 'k', long, value_name = "IDENTIFIER", group = "update_from")]
        from_record: Option<RecordId>,
        /// How to resolve conflicting field values.
        #[arg(
            short = 'n',
            long,
            value_enum,
            default_value_if("no_interactive", ArgPredicate::IsPresent, "prefer-current"),
            default_value_t
        )]
        on_conflict: OnConflict,
        /// Retrieve new data if the record is deleted.
        #[arg(long)]
        revive: bool,
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
#[derive(Debug, Subcommand)]
pub enum AliasCommand {
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
    /// Reset an alias to refer to a new record.
    Reset {
        /// The name of the existing alias.
        #[arg(value_parser = with_short_err::<Alias>)]
        alias: Alias,
        /// What the alias should point to.
        target: RecordId,
    },
}

pub enum ReadOnlyInvalid {
    Command(&'static str),
    Argument(&'static str),
}

impl Cli {
    /// Perform argument validation that Clap cannot do.
    pub fn validate(&self) {
        if self.read_only
            && let Err(invalid) = self.command.validate_read_only_compatibility()
        {
            let mut cmd = Self::command();
            let (name, s) = match invalid {
                ReadOnlyInvalid::Command(s) => ("subcommand", s),
                ReadOnlyInvalid::Argument(s) => ("argument", s),
            };
            let err_msg = format!(
                "the {} '{}' cannot be used in read-only mode (enabled by '{}')",
                name,
                s.stylize().yellow(),
                "--read-only".stylize().yellow(),
            );
            cmd.error(ErrorKind::ArgumentConflict, err_msg).exit();
        }
    }
}

impl UtilCommand {
    /// Check if the command is read-only compatible.
    pub fn validate_read_only_compatibility(&self) -> Result<(), ReadOnlyInvalid> {
        match self {
            Self::List { .. } | Self::Check { fix: false } => Ok(()),
            Self::Check { fix: true, .. } => Err(ReadOnlyInvalid::Argument("--fix")),
            Self::Optimize => Err(ReadOnlyInvalid::Command("util optimize")),
            Self::Evict { .. } => Err(ReadOnlyInvalid::Command("util evict")),
        }
    }
}

impl Command {
    /// Check if the command is read-only compatible.
    pub fn validate_read_only_compatibility(&self) -> Result<(), ReadOnlyInvalid> {
        // exhaustive matching so that there is a compile error if the `Cli` struct changes
        let invalid_cmd = match self {
            Self::Get { .. }
            | Self::Info { .. }
            | Self::Source { .. }
            | Self::Completions { .. }
            | Self::DefaultConfig
            | Self::Find { .. }
            | Self::Log { .. }
            | Self::Path { mkdir: false, .. } => return Ok(()),
            Self::Path { mkdir: true, .. } => return Err(ReadOnlyInvalid::Argument("--mkdir")),
            Self::Alias { .. } => "alias",
            Self::Attach { .. } => "attach",
            Self::Delete { .. } => "delete",
            Self::Import { .. } => "import",
            Self::Local { .. } => "local",
            Self::Update { .. } => "update",
            Self::Edit { .. } => "edit",
            Self::Hist { .. } => "hist",
            Self::Util { util_command } => return util_command.validate_read_only_compatibility(),
        };
        Err(ReadOnlyInvalid::Command(invalid_cmd))
    }
}

/// Commands to manipulate version history.
#[derive(Debug, Subcommand)]
pub enum HistCommand {
    /// Clean up edit history without impacting the active record.
    Prune {
        #[command(subcommand)]
        prune_command: PruneCommand,
    },
    /// Redo previously undone changes.
    ///
    /// If no arguments are provided, this will redo the most recent change.
    ///
    /// The optional INDEX refers to the 0-indexed change, ordered from oldest to newest.
    /// Negative values of INDEX are permitted and count backwards from newest to oldest.
    ///
    /// For example, INDEX 0 is the oldest change and INDEX -1 is the newest change.
    ///
    /// View divergent changes using `autobib log --tree`.
    Redo {
        /// The identifier for the redo operation.
        identifier: RecordId,
        /// The index of the redo, ordered from oldest to newest.
        index: Option<isize>,
        /// Redo beyond a deleted state.
        #[arg(short, long)]
        revive: bool,
    },
    /// Set the active version to a specific revision.
    Reset {
        /// The identifier for the reset operation.
        identifier: RecordId,
        /// Set using a revision number.
        #[arg(long, group = "reset_target")]
        rev: Option<RevisionId>,
        /// Set to the lastest state with modification time preceding this date-time.
        ///
        /// This is a RFC3339 date-time, so make sure to indicate the timezone as well.
        #[arg(long, group = "reset_target")]
        before: Option<DateTime<Local>>,
    },
    /// Insert new data for a deleted record, concealing any prior changes.
    ///
    /// Usually you want to use `autobib hist undo`, and then `edit` the resulting record.
    /// This method is useful if there is no prior state or if you want to intentionally conceal prior changes.
    ///
    /// If no arguments are specified, you will be prompted to edit the local record before adding it to the
    /// database. If the terminal is non-interactive or `--no-interactive` is set, this will insert
    /// a default value with no contents.
    ///
    /// You can provide BibTeX data from a file with the `--from-bibtex` option, or by providing
    /// values using `--with-entry-type` and `--with-field`.
    ///
    /// The `--with-entry-type` or `--with-field` values will override any
    /// values present in the data read from the BibTeX file.
    Revive {
        /// The identifier for the revive operation.
        identifier: RecordId,
        /// Create the record using the provided BibTeX data.
        #[arg(short = 'b', long, value_name = "PATH", group = "input")]
        from_bibtex: Option<PathBuf>,
        /// Set the entry type.
        #[arg(long, value_name = "ENTRY_TYPE")]
        with_entry_type: Option<EntryType>,
        /// Set specific field values using BibTeX field syntax
        #[arg(long, value_name = "FIELD_KEY={VALUE}")]
        with_field: Vec<SetFieldCommand>,
    },
    /// Move the database back in time.
    ///
    /// This is the same as calling `autobib reset --before` on every active entry in the database with
    /// modification greater than the provided time.
    ///
    /// Use caution! The modification time may not correspond to the database state at the provided
    /// date-time if you have used `autobib undo/redo/reset`, since these methods only change the
    /// active state without introducing new changes. Your old data will still be retrievable, but
    /// it could require a lot of work to unwind the changes.
    RewindAll {
        /// The datetime to rewind to.
        before: DateTime<Local>,
    },
    /// Show all database changes in descending order by time.
    Show {
        /// Only show LIMIT most recent changes
        #[arg(long, value_name = "LIMIT")]
        limit: Option<u32>,
    },
    /// Undo the most recent change associated with an identifier.
    Undo {
        /// The identifier for the undo operation.
        identifier: RecordId,
        /// Undo into a deleted state.
        #[arg(short, long)]
        delete: bool,
    },
    /// Void a record.
    ///
    /// A voided record is equivalent to a record which is not in the database, but the previous
    /// history is still recoverable.
    Void {
        /// The identifier to void.
        identifier: RecordId,
    },
}

/// Permanently remove edit history without impacting the active record.
///
/// These operations are performed in bulk on the entire database, so if your database is very
/// large they can take a while to run, particularly the `autobib prune outdated` operation.
#[derive(Debug, Subcommand)]
pub enum PruneCommand {
    /// Prune all inactive entries.
    All,
    /// Prune inactive deletion and void markers.
    Deleted,
    /// Prune entries which are not a descendent of an active entry.
    Outdated {
        /// Also keep entries that are a descendent of a level `n` ancestor of the active entry.
        #[arg(long, default_value_t = 0)]
        retain: u32,
    },
}

/// Utilities to manage database.
#[derive(Debug, Subcommand)]
pub enum UtilCommand {
    /// Check database for errors.
    Check {
        /// Attempt to fix errors, printing any errors which could not be fixed.
        #[arg(short, long)]
        fix: bool,
    },
    /// Optimize database to (potentially) reduce storage size.
    Optimize,
    /// Clear all local caches.
    Evict {
        /// Clear cached items which are at least `seconds` old.
        #[arg(long)]
        max_age: Option<u32>,
    },
    /// List all valid keys.
    List {
        /// Only list the canonical keys.
        #[arg(short, long)]
        canonical: bool,
    },
}
