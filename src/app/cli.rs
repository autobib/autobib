use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    str::FromStr,
};

use anyhow::Result;
use clap::{
    CommandFactory, Parser, Subcommand, ValueEnum, builder::ArgPredicate, error::ErrorKind,
};
use clap_complete::aot::Shell;
use clap_verbosity_flag::{Verbosity, WarnLevel};
use crossterm::style::Stylize;

use crate::{
    cite_search::SourceFileType,
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
        citation_key: RecordId,
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
        ///
        /// Deletion will fail if user confirmation is required,the program is running
        /// non-interactively, and this option is not set.
        #[arg(short, long)]
        force: bool,
    },
    /// Edit existing records.
    ///
    /// Edit an existing record using your $EDITOR. This will open a BibTeX file with the
    /// contents of the record. Updating the fields or the entry type will change the underlying
    /// data, and updating the entry key will create a new alias for the record.
    ///
    /// Some non-interactive edit methods are supported. These can be used along with the
    /// `--no-interactive` option to modify records without opening your $EDITOR:
    ///
    /// `--normalize-whitespace` converts whitespace blocks into a single ASCII space.
    ///
    /// `--set-eprint` accepts a list of field keys, and sets the "eprint" and
    ///   "eprinttype" BibTeX fields from the first field key which is present in the record.
    Edit {
        /// The citation key(s) to edit.
        citation_keys: Vec<RecordId>,
        /// Normalize whitespace.
        #[arg(long)]
        normalize_whitespace: bool,
        /// Set "eprint" and "eprinttype" BibTeX fields from provided fields.
        #[arg(long, value_delimiter = ',', value_name = "FIELD_NAME")]
        set_eprint: Vec<String>,
        /// Strip trailing journal series
        #[arg(long)]
        strip_journal_series: bool,
    },
    /// Search for a citation key.
    ///
    /// Open an interactive picker to search for a given citation key. The lines in the
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
    /// Retrieve records given citation keys.
    Get {
        /// The citation keys to retrieve.
        citation_keys: Vec<RecordId>,
        /// Write output to file.
        #[arg(short, long, group = "output", value_name = "PATH")]
        out: Option<PathBuf>,
        /// Append new entries to the output, skipping existing entries.
        #[arg(short, long, requires = "out")]
        append: bool,
        /// Retrieve records but do not output BibTeX or check the validity of citation keys.
        #[arg(long, group = "output")]
        retrieve_only: bool,
        /// Ignore null records and aliases.
        #[arg(long)]
        ignore_null: bool,
    },
    /// Import records from a BibTeX file.
    Import {
        /// The BibTeX file(s) from which to import.
        targets: Vec<PathBuf>,
        #[arg(short = 'm', long, value_enum, default_value_t)]
        /// The type of import to perform.
        mode: ImportMode,
        /// How to resolve conflicting field values.
        #[arg(
            short = 'n',
            long,
            value_enum,
            default_value_if("no_interactive", ArgPredicate::IsPresent, "prefer-current"),
            default_value_t
        )]
        on_conflict: OnConflict,
        /// Never create aliases.
        #[arg(short = 'A', long)]
        no_alias: bool,
        /// Replace colons in entry keys with a new string.
        #[arg(long, value_name = "STR")]
        replace_colons: Option<String>,
        /// Print entries which could not be imported
        #[arg(long)]
        log_failures: bool,
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
        /// Create local record from BibTeX file.
        #[arg(short, long, value_name = "PATH", group = "input")]
        from: Option<PathBuf>,
        /// Rename an existing local record.
        #[arg(long, value_name = "EXISTING_ID", group = "input")]
        rename_from: Option<String>,
        /// Do not create the alias `<ID>` for `local:<ID>`.
        #[arg(short = 'A', long)]
        no_alias: bool,
    },
    /// Combine multiple records.
    Merge {
        /// The highest priority record which will be retained.
        into: RecordId,
        /// Records to be merged.
        from: Vec<RecordId>,
        /// How to resolve conflicting field values.
        #[arg(
            short = 'n',
            long,
            value_enum,
            default_value_if("no_interactive", ArgPredicate::IsPresent, "prefer-current"),
            default_value_t
        )]
        on_conflict: OnConflict,
    },
    /// Show attachment directory associated with record.
    Path {
        /// Show path for this key.
        citation_key: RecordId,
        /// Also create the directory.
        #[arg(short, long)]
        mkdir: bool,
    },
    /// Generate records by searching for citation keys inside files.
    ///
    /// This is essentially a call to `autobib get`, except with a custom search which attempts
    /// to find citation keys inside the provided file(s). The search method depends on the file
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
        /// Retrieve records but do not output BibTeX or check the validity of citation keys.
        #[arg(long, group = "output")]
        retrieve_only: bool,
        /// Only print the citation keys which were found (sorted and deduplicated).
        #[arg(long, group = "output")]
        print_keys: bool,
        /// Skip a citation key (if present).
        #[arg(short, long, value_name = "CITATION_KEYS")]
        skip: Vec<RecordId>,
        /// Skip citation keys which are present in the provided file(s).
        #[arg(long, value_name = "PATH")]
        skip_from: Vec<PathBuf>,
        /// Override file type detection for skip files.
        #[arg(long, value_name = "FILETYPE")]
        skip_file_type: Option<SourceFileType>,
        /// Ignore null records and aliases.
        #[arg(long)]
        ignore_null: bool,
    },
    /// Update data associated with an existing citation key.
    ///
    /// By default, you will be prompted if there is a conflict between the current and incoming
    /// records.
    ///
    /// To override this behaviour, use `-p current` or `-p incoming`.
    /// If the terminal is not interactive or the `--no-interactive` global option is set, this
    /// will result in an error if the `-p current` or `-p incoming` is not explicitly set.
    Update {
        /// The citation key to update.
        citation_key: RecordId,
        /// Read update data from local path.
        #[arg(short, long, value_name = "PATH")]
        from: Option<PathBuf>,
        /// How to resolve conflicting field values.
        #[arg(
            short = 'n',
            long,
            value_enum,
            default_value_if("no_interactive", ArgPredicate::IsPresent, "prefer-current"),
            default_value_t
        )]
        on_conflict: OnConflict,
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
            | Self::Path { mkdir: false, .. } => return Ok(()),
            Self::Path { mkdir: true, .. } => return Err(ReadOnlyInvalid::Argument("--mkdir")),
            Self::Alias { .. } => "alias",
            Self::Attach { .. } => "attach",
            Self::Delete { .. } => "delete",
            Self::Import { .. } => "import",
            Self::Local { .. } => "local",
            Self::Merge { .. } => "merge",
            Self::Update { .. } => "update",
            Self::Edit { .. } => "edit",
            Self::Util { util_command } => return util_command.validate_read_only_compatibility(),
        };
        Err(ReadOnlyInvalid::Command(invalid_cmd))
    }
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
