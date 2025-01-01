mod cli;
mod edit;
mod path;
mod picker;
mod retrieve;
mod write;

use std::{
    collections::HashSet,
    fs::{create_dir_all, File, OpenOptions},
    io::{copy, Read},
    iter::once,
    path::Path,
    str::FromStr,
};

use anyhow::{bail, Result};
use etcetera::{choose_app_strategy, AppStrategy, AppStrategyArgs};
use itertools::Itertools;
use serde_bibtex::token::is_entry_key;

use crate::{
    cite_search::{get_citekeys, SourceFileType},
    config,
    db::{
        binary_format_version, schema_version,
        state::{RecordIdState, RemoteIdState, RowData},
        DeleteAliasResult, EvictionConstraint, RecordData, RecordDatabase, RenameAliasResult,
    },
    error::AliasErrorKind,
    http::HttpClient,
    logger::{error, info, suggest, warn},
    normalize::Normalize,
    record::Record,
    record::{get_record_row, Alias, RecordId, RemoteId},
    term::Confirm,
};

use self::{
    cli::{AliasCommand, InfoReportType, UpdateMode, UtilCommand},
    edit::{edit_record_and_update, merge_record_data},
    path::{data_from_path_or_default, data_from_path_or_remote, get_attachment_dir},
    picker::choose_canonical_id,
    retrieve::{filter_and_deduplicate_by_canonical, retrieve_and_validate_entries},
    write::output_entries,
};

pub use self::cli::{Cli, Command};

/// Run the CLI.
pub fn run_cli(cli: Cli) -> Result<()> {
    info!("SQLite version: {}", rusqlite::version());
    info!("Autobib version: {}", env!("CARGO_PKG_VERSION"));
    info!("Database binary data version: {}", binary_format_version());
    info!("Database schema version: {}", schema_version());

    let strategy = choose_app_strategy(AppStrategyArgs {
        top_level_domain: "org".to_owned(),
        author: env!("CARGO_PKG_NAME").to_owned(),
        app_name: env!("CARGO_PKG_NAME").to_owned(),
    })?;

    let data_dir = strategy.data_dir();

    // Open or create the database
    let mut record_db = if let Some(db_path) = cli.database {
        // at a user-provided path
        info!("Using user-provided database file '{}'", db_path.display());
        if let Some(db_parent) = db_path.parent() {
            create_dir_all(db_parent)?;
        }
        RecordDatabase::open(db_path)?
    } else {
        // at the default path
        let default_db_path = data_dir.join("records.db");
        info!(
            "Using default database file '{}'",
            default_db_path.display()
        );
        create_dir_all(&data_dir)?;
        RecordDatabase::open(default_db_path)?
    };

    let (config_path, missing_ok) = cli.config.map_or_else(
        || (strategy.config_dir().join("config.toml"), true),
        |path| (path, false),
    );

    // Initialize the reqwest Client
    let builder = HttpClient::default_builder();
    let client = HttpClient::new(builder)?;

    // Run the cli
    match cli.command {
        Command::Alias { alias_command } => match alias_command {
            AliasCommand::Add { alias, target } => {
                info!("Creating alias '{alias}' for '{target}'");
                let cfg = config::load(&config_path, missing_ok)?;
                let (_, row) = get_record_row(&mut record_db, target, &client, &cfg)?
                    .exists_or_commit_null("Cannot create alias for")?;
                if !row.add_alias(&alias)? {
                    error!("Alias already exists: '{alias}'");
                }
                row.commit()?;
            }
            AliasCommand::Delete { alias } => {
                info!("Deleting alias '{alias}'");
                match record_db.delete_alias(&alias)? {
                    DeleteAliasResult::Deleted => {}
                    DeleteAliasResult::Missing => {
                        bail!("Could not delete alias which does not exist: '{alias}'")
                    }
                }
            }
            AliasCommand::Rename { alias, new } => {
                info!("Rename alias '{alias}' to '{new}'");
                match record_db.rename_alias(&alias, &new)? {
                    RenameAliasResult::Renamed => {}
                    RenameAliasResult::TargetExists => {
                        bail!("Citation key already exists: '{new}'");
                    }
                }
            }
        },
        Command::Attach {
            citation_key,
            file,
            rename,
            force,
        } => {
            // Extend with the filename.
            let cfg = config::load(&config_path, missing_ok)?;
            let (record, row) = get_record_row(&mut record_db, citation_key, &client, &cfg)?
                .exists_or_commit_null("Cannot attach file for")?;
            row.commit()?;
            let mut target = get_attachment_dir(&record.canonical, &data_dir, cli.attachments_dir)?;

            create_dir_all(&target)?;

            // Try to open the source file first, since this will reduce the number of redundant
            // errors.
            let mut source_file = File::open(&file)?;

            // determine the target filename, either by parsing from the 'rename' value or
            // defaulting to the filename of the source file
            target.push(match rename {
                None => {
                    if let Some(name) = file.file_name() {
                        name
                    } else {
                        bail!("Source file must not be a directory");
                    }
                }
                Some(ref rename) => {
                    match (rename.parent().and_then(Path::to_str), rename.file_name()) {
                        // rename.parent() returns Some("") for relative paths with one component; see
                        //  https://doc.rust-lang.org/stable/std/path/struct.Path.html#method.parent
                        (Some(""), Some(filename)) => filename,
                        _ => {
                            bail!("Renamed value must be a relative path with one component");
                        }
                    }
                }
            });

            let mut opts = OpenOptions::new();
            opts.write(true);
            if !force {
                opts.create_new(true);
            }

            let mut target_file = opts.open(&target)?;
            copy(&mut source_file, &mut target_file)?;
        }
        Command::Completions { shell: _ } => {
            unreachable!("Request for completions script should have been handled earlier and the program should have exited then.");
        }
        Command::Delete {
            citation_keys,
            force,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;
            let deduplicated = filter_and_deduplicate_by_canonical(
                citation_keys.into_iter(),
                &mut record_db,
                force,
                |remote_id, null_row| {
                    null_row.commit()?;
                    error!("Null record found for '{remote_id}'");
                    suggest!("Delete null records using `autobib util evict`.");
                    Ok(())
                },
                &cfg,
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
            let cfg = config::load(&config_path, missing_ok)?;
            let (mut record, row) = get_record_row(&mut record_db, citation_key, &client, &cfg)?
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
            if cli.no_interactive {
                bail!("`autobib find` cannot run in non-interactive mode");
            }

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
            let cfg = config::load(&config_path, missing_ok)?;
            // Collect all entries which are not null
            let valid_entries = retrieve_and_validate_entries(
                citation_keys.into_iter(),
                &mut record_db,
                &client,
                retrieve_only,
                ignore_null,
                &cfg,
            );

            if !retrieve_only {
                output_entries(out.as_ref(), valid_entries)?;
            }
        }
        Command::Info {
            citation_key,
            report,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;
            match record_db.state_from_record_id(citation_key, &cfg.alias_transform)? {
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
                                if is_entry_key(&record_id) {
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
                            if !is_entry_key(&record_id) {
                                error!("Invalid BibTeX: {record_id}");
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
                RecordIdState::UnknownRemoteId(maybe_normalized, missing) => {
                    missing.commit()?;
                    bail!("Cannot obtain report for record not in database: {maybe_normalized}");
                }
                RecordIdState::UndefinedAlias(alias) => {
                    bail!("Cannot obtain report for undefined alias: '{alias}'");
                }

                RecordIdState::InvalidRemoteId(err) => bail!("{err}"),
            }
        }
        Command::Local {
            id,
            from,
            rename_from,
            no_alias,
        } => {
            let alias = match Alias::from_str(&id) {
                Ok(alias) => alias,
                Err(e) => match e.kind {
                    AliasErrorKind::Empty => {
                        bail!("local sub-id must contain non-whitespace characters")
                    }
                    AliasErrorKind::IsRemoteId => bail!("local sub-id must not contain a colon"),
                },
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
        Command::Merge {
            into,
            from,
            prefer_current,
            prefer_incoming,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;
            let (existing, row) = get_record_row(&mut record_db, into, &client, &cfg)?
                .exists_or_commit_null("Cannot merge into")?;

            let new_data: Vec<RowData> = from
                .iter()
                // filter keys which cannot be resolved or are equivalent to the merge target
                .filter_map(|record_id| {
                    // this implementation is automatically de-duplicating since an earlier merge
                    // will result in a CitationKey entry which points to the row, which is then
                    // dropped automatically.
                    row.absorb(record_id, || {
                        error!("Skipping key '{record_id}' which does not exist in the database!");
                    })
                    .transpose()
                })
                .collect::<Result<_, rusqlite::Error>>()?;

            // merge data
            let mut existing_record = RecordData::from(&existing.data);
            merge_record_data(
                UpdateMode::from_flags(cli.no_interactive, prefer_current, prefer_incoming),
                &mut existing_record,
                new_data.iter().map(|row_data| &row_data.data),
                &existing.key,
            )?;

            // update the row data with the modified data
            row.save_to_changelog()?;
            row.update_row_data(&(&existing_record).into())?;
            row.commit()?;
        }
        Command::Path {
            citation_key,
            mkdir,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;
            // Extend with the filename.
            let (record, row) = get_record_row(&mut record_db, citation_key, &client, &cfg)?
                .exists_or_commit_null("Cannot show directory for")?;
            row.commit()?;
            let mut target = get_attachment_dir(&record.canonical, &data_dir, cli.attachments_dir)?;

            if mkdir {
                create_dir_all(&target)?;
            }

            // This appends a `/` or `\` when printing, as platform appropriate, to be clear to the
            // user that this is a directory
            target.push("");

            println!("{}", target.display());
        }
        Command::Source {
            paths,
            file_type,
            out,
            retrieve_only,
            ignore_null,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;
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
                &cfg,
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
        } => {
            let cfg = config::load(&config_path, missing_ok)?;
            match record_db.state_from_record_id(citation_key, &cfg.alias_transform)? {
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
                    let mut existing_record = RecordData::from(&existing_raw_data);
                    merge_record_data(
                        UpdateMode::from_flags(cli.no_interactive, prefer_current, prefer_incoming),
                        &mut existing_record,
                        once(&new_raw_data),
                        &citation_key,
                    )?;
                    row.save_to_changelog()?;
                    row.update_row_data(&(&existing_record).into())?;
                    row.commit()?;
                }
                RecordIdState::NullRemoteId(mapped_remote_id, null_row) => {
                    match data_from_path_or_remote(from, mapped_remote_id.mapped, &client) {
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
                RecordIdState::UnknownRemoteId(maybe_normalized, missing) => {
                    error!("Record does not exist in database: {maybe_normalized}");
                    if !maybe_normalized.mapped.is_local() {
                        suggest!("Use `autobib get` to retrieve record");
                    }
                    missing.commit()?;
                }
                RecordIdState::UndefinedAlias(alias) => {
                    bail!("Undefined alias: '{alias}'");
                }
                RecordIdState::InvalidRemoteId(err) => bail!("{err}"),
            }
        }
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

                info!("Validating configuration.");
                config::validate(&config_path)?;
            }
            UtilCommand::Evict {
                regex,
                before,
                after,
            } => {
                let constraints = EvictionConstraint::default()
                    .regex(&regex)
                    .before(&before)
                    .after(&after);

                record_db.evict_cache(&constraints)?;
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
