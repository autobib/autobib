use std::{path::PathBuf, str::FromStr};

use anyhow::Result;
use chrono::{DateTime, Local};
use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::aot::Shell;
use clap_verbosity_flag::{Verbosity, WarnLevel};

use crate::{
    cite_search::SourceFileType,
    error::ShortError,
    record::{Alias, RecordId},
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Use record database.
    #[arg(short, long, value_name = "PATH", env = "AUTOBIB_DATABASE_PATH")]
    pub database: Option<PathBuf>,
    /// Use configuration file.
    #[arg(short, long, value_name = "PATH", env = "AUTOBIB_CONFIG_PATH")]
    pub config: Option<PathBuf>,
    /// Do not require user action.
    ///
    /// This option is set automatically if the standard input is not a terminal.
    #[arg(short = 'I', long, global = true)]
    pub no_interactive: bool,
    /// Use directory for attachments.
    #[arg(long, value_name = "PATH", env = "AUTOBIB_ATTACHMENTS_DIRECTORY")]
    pub attachments_dir: Option<PathBuf>,
    #[command(flatten)]
    pub verbose: Verbosity<WarnLevel>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Copy, Clone, ValueEnum, Default)]
pub enum InfoReportType {
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
pub enum UpdateMode {
    PreferCurrent,
    PreferIncoming,
    Prompt,
}

impl UpdateMode {
    pub fn from_flags(no_interactive: bool, prefer_current: bool, prefer_incoming: bool) -> Self {
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
pub enum Command {
    /// Manage aliases.
    Alias {
        #[command(subcommand)]
        alias_command: AliasCommand,
    },
    /// Attach files.
    ///
    /// Add new files to the directory associated with a record, as determined by the `path`
    /// subcommand. The original file is copied to the new directory, or can be renamed
    /// with the `--rename` option.
    Attach {
        /// The record to associate the files with.
        citation_key: RecordId,
        /// The path to the file to add.
        file: PathBuf,
        /// Rename the file.
        #[arg(short, long)]
        rename: Option<PathBuf>,
        /// Overwrite existing files.
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
        /// Fields to search (e.g. author, title), delimited by a comma.
        #[arg(short, long, value_delimiter = ',', default_value = "author,title")]
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
    /// Combine multiple records.
    Merge {
        /// The highest priority record which will be retained.
        into: RecordId,
        /// Records to be merged.
        from: Vec<RecordId>,
        /// Keep the current value without prompting in the event of a conflict.
        #[arg(long, group = "update-mode")]
        prefer_current: bool,
        /// Update with the incoming value without prompting in the event of a conflict.
        #[arg(long, group = "update-mode")]
        prefer_incoming: bool,
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
    /// option; `--prefer-incoming` takes precedence over `--prefer-current`.
    /// The `--no-interactive` global option implies `--prefer-current`.
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

/// Utilities to manage database.
#[derive(Subcommand)]
pub enum UtilCommand {
    /// Check database for errors.
    Check {
        /// Attempt to fix errors, printing any errors which could not be fixed.
        #[arg(short, long)]
        fix: bool,
    },
    /// Clear caches which match all of the provided conditions.
    Evict {
        /// Clear cached items with citation keys matching this regex.
        ///
        /// The regex syntax is documented at <https://docs.rs/regex/latest/regex/#syntax>
        #[arg(short, long)]
        regex: Option<String>,
        /// Clear cached items predating the provided time.
        #[arg(short, long)]
        before: Option<DateTime<Local>>,
        /// Clear cached items following the provided time.
        #[arg(short, long)]
        after: Option<DateTime<Local>>,
    },
    /// List all valid keys.
    List {
        /// Only list the canonical keys.
        #[arg(short, long)]
        canonical: bool,
    },
}