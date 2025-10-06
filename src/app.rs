mod cli;
mod edit;
mod import;
mod path;
mod picker;
mod retrieve;
mod source;
mod write;

use std::{
    collections::{BTreeSet, HashSet},
    fs::{File, OpenOptions, create_dir_all},
    io::{Read, Seek, copy},
    iter::once,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{Result, bail};
use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};
use itertools::Itertools;
use serde_bibtex::token::is_entry_key;

use crate::{
    CitationKey,
    cite_search::{SourceFileType, get_citekeys},
    config,
    db::{
        DeleteAliasResult, RecordDatabase, RenameAliasResult,
        state::{ExistsOrUnknown, RecordIdState, RowData},
        user_version,
    },
    entry::{Entry, EntryKey, RawRecordData, RecordData},
    error::AliasErrorKind,
    http::{BodyBytes, Client},
    logger::{debug, error, info, suggest, warn},
    normalize::{Normalization, Normalize},
    record::{Alias, Record, RecordId, RemoteId, get_record_row},
    term::Confirm,
};

use self::{
    cli::{AliasCommand, FindMode, ImportMode, InfoReportType, UpdateMode, UtilCommand},
    edit::{edit_record_and_update, merge_record_data},
    path::{
        data_from_path_or_default, data_from_path_or_remote, get_attachment_dir,
        get_attachment_root,
    },
    picker::{choose_attachment, choose_attachment_path, choose_canonical_id},
    retrieve::{
        filter_and_deduplicate_by_canonical, retrieve_and_validate_entries,
        retrieve_entries_read_only,
    },
    write::{init_outfile, output_entries, output_keys},
};

pub use self::cli::{Cli, Command, ReadOnlyInvalid};

/// Run the CLI.
pub fn run_cli<C: Client>(cli: Cli, client: &C) -> Result<()> {
    info!(
        "Autobib version: {} (database version: {})",
        env!("CARGO_PKG_VERSION"),
        user_version()
    );
    info!("SQLite version: {}", rusqlite::version());

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
        RecordDatabase::open(db_path, cli.read_only)?
    } else {
        // at the default path
        let default_db_path = data_dir.join("records.db");
        info!(
            "Using default database file '{}'",
            default_db_path.display()
        );
        create_dir_all(&data_dir)?;
        RecordDatabase::open(default_db_path, cli.read_only)?
    };
    info!("On-disk database version: {}", record_db.user_version()?);

    let (config_path, missing_ok) = cli.config.map_or_else(
        || (strategy.config_dir().join("config.toml"), true),
        |path| (path, false),
    );

    // Run the cli
    match cli.command {
        Command::Alias { alias_command } => match alias_command {
            AliasCommand::Add { alias, target } => {
                info!("Creating alias '{alias}' for '{target}'");
                let cfg = config::load(&config_path, missing_ok)?;
                let (_, row) = get_record_row(&mut record_db, target, client, &cfg)?
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
            /// Determine the target filename from the `rename` value (if any), and otherwise
            /// use the provided fallback
            fn use_rename_or_fallback(
                target: &mut PathBuf,
                rename: Option<PathBuf>,
                fallback: Option<&std::ffi::OsStr>,
            ) -> Result<(), anyhow::Error> {
                target.push(match rename {
                    None => {
                        if let Some(name) = fallback {
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
                Ok(())
            }

            // Extend with the filename.
            let cfg = config::load(&config_path, missing_ok)?;
            let (record, row) = get_record_row(&mut record_db, citation_key, client, &cfg)?
                .exists_or_commit_null("Cannot attach file for")?;
            row.commit()?;
            let mut target = get_attachment_dir(&data_dir, cli.attachments_dir, &record.canonical)?;

            let mut opts = OpenOptions::new();
            opts.write(true);
            if !force {
                opts.create_new(true);
            }

            // create the destination directory
            create_dir_all(&target)?;

            match ureq::http::Uri::try_from(&file) {
                Ok(uri) if uri.scheme().is_some() => {
                    // In the URI case, defer the network request for as long as possible.

                    // This is the correct way to read the final component from a URI path; see
                    // https://datatracker.ietf.org/doc/html/rfc3986#section-3.3
                    let path = uri.path();
                    let name = match uri.path().rsplit_once('/') {
                        Some((_, name)) => name,
                        None => path,
                    };
                    if name.is_empty() {
                        bail!(
                            "Could not determine filename from URL. Use `--rename` to manually specify a name."
                        );
                    }

                    use_rename_or_fallback(&mut target, rename, Some(std::ffi::OsStr::new(name)))?;

                    info!("Downloading file from: {uri}");
                    let response = client.get(uri)?;
                    let mut body = match response.status() {
                        ureq::http::StatusCode::OK => response.into_body(),
                        c => bail!("Failed to download file: {c}"),
                    };
                    let mut target_file = opts.open(dbg!(&target))?;
                    copy(&mut body.as_reader(), &mut target_file)?;
                }
                _ => {
                    let file = PathBuf::from(file);

                    // Try to open the source file first, since this will reduce the number of redundant
                    // errors.
                    let mut source_file = File::open(&file)?;

                    use_rename_or_fallback(&mut target, rename, file.file_name())?;

                    info!("Copying file from: {}", file.display());
                    let mut target_file = opts.open(&target)?;
                    copy(&mut source_file, &mut target_file)?;
                }
            }
        }
        Command::Completions { shell: _ } => {
            unreachable!(
                "Request for completions script should have been handled earlier and the program should have exited then."
            );
        }
        Command::DefaultConfig => {
            config::write_default(&mut std::io::stdout())?;
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
                match record_db.state_from_remote_id(&canonical)?.exists() {
                    Some(row) => {
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
                                    error!(
                                        "Record with canonical identifier '{canonical}' has associated keys which are not requested for deletion: {}",
                                        unreferenced.join(", ")
                                    );
                                    suggest!("Re-run with `--force` to delete anyway.");
                                    continue;
                                } else {
                                    // interactive: prompt for deletion
                                    eprintln!(
                                        "Deleting record with canonical identifier '{canonical}' will also delete associated keys:"
                                    );
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
                    }
                    _ => {
                        error!(
                            "Database changed during deletion operation! Record {canonical} is no longer present in the database."
                        );
                    }
                }
            }
        }
        Command::Edit {
            citation_keys,
            normalize_whitespace,
            set_eprint,
            strip_journal_series,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;
            let nl = Normalization {
                normalize_whitespace,
                set_eprint,
                strip_journal_series,
            };

            for key in citation_keys {
                let (
                    Record {
                        key,
                        data,
                        canonical,
                    },
                    row,
                ) = get_record_row(&mut record_db, key, client, &cfg)?
                    .exists_or_commit_null("Cannot edit")?;

                if cli.no_interactive {
                    if nl.is_identity() {
                        warn!("Terminal is non-interactive and no edit action specified!");
                        row.commit()?;
                    } else {
                        // non-interactive so we only apply the normalizations and update the data
                        // if anything changed
                        let mut editable_data = RecordData::from_entry_data(&data);
                        if editable_data.normalize(&nl) {
                            row.save_to_changelog()?;
                            row.update_entry_data(&editable_data)?;
                        }
                        row.commit()?;
                    }
                } else {
                    let mut editable_data = RecordData::from_entry_data(&data);
                    let changed = editable_data.normalize(&nl);
                    let entry_key =
                        EntryKey::try_new(key).unwrap_or_else(|_| EntryKey::placeholder());
                    edit_record_and_update(
                        &row,
                        Entry::new(entry_key, editable_data),
                        changed,
                        canonical,
                    )?;
                    row.commit()?;
                }
            }
        }
        Command::Find {
            fields,
            entry_type,
            attachments,
            records,
            all_fields,
        } => {
            let find_mode = FindMode::from_flags(attachments, records);

            if cli.no_interactive {
                bail!("`autobib find` cannot run in non-interactive mode");
            }

            let cfg = config::load(&config_path, missing_ok)?;

            match find_mode {
                FindMode::Attachments => {
                    let mut picker = choose_attachment_path(
                        record_db,
                        fields,
                        entry_type,
                        all_fields,
                        get_attachment_root(&data_dir, cli.attachments_dir)?,
                        cfg.find.ignore_hidden,
                        Path::is_file,
                    );
                    match picker.pick()? {
                        Some(data) => {
                            if data.attachments.len() > 1 {
                                // if there are multiple attachments, open the picker again to
                                // select an attachment
                                //
                                // unfortunately the borrow here is unavoidable since `nucleo` does
                                // not allow passing ownership of the underlying item buffer back
                                // to the caller when complete.
                                let mut attachment_picker = choose_attachment(data);
                                match attachment_picker.pick()? {
                                    Some(dir_entry) => {
                                        println!("{}", dir_entry.path().display());
                                    }
                                    None => error!("No attachment selected."),
                                }
                            } else {
                                println!("{}", data.attachments.first().path().display());
                            };
                        }
                        None => error!("No record selected."),
                    }
                }
                FindMode::CanonicalId => {
                    let (mut picker, handle) =
                        choose_canonical_id(record_db, fields, entry_type, all_fields);
                    match picker.pick()? {
                        Some(row_data) => {
                            let cfg = config::load(&config_path, missing_ok)?;
                            if !cfg.preferred_providers.is_empty() {
                                // get a key from the preferred provider if possible
                                let mut record_db =
                                    handle.join().expect("Thread should not have panicked")?;
                                match record_db
                                    .state_from_remote_id(&row_data.canonical)?
                                    .exists()
                                {
                                    Some(row) => {
                                        // try to find a referencing key with the expected provider
                                        let referencing_ids = row.get_referencing_remote_ids()?;
                                        for provider in cfg.preferred_providers {
                                            if let Some(remote_id) = referencing_ids
                                                .iter()
                                                .find(|id| id.provider() == provider)
                                            {
                                                println!("{remote_id}");
                                                return Ok(());
                                            }
                                        }
                                    }
                                    _ => {
                                        bail!("Record deleted while picker was running!");
                                    }
                                };
                            }

                            // if there are no preferred providers or none matched, just print
                            // the canonical identifier
                            println!("{}", row_data.canonical);
                        }
                        None => error!("No item selected."),
                    }
                }
            }
        }
        Command::Get {
            citation_keys,
            out,
            append,
            retrieve_only,
            ignore_null,
        } => {
            let mut outfile = init_outfile(out, append)?;

            // Initialize the skipped keys to contain keys already present in the outfile (if
            // appending)
            let mut skipped_keys: HashSet<RecordId> = HashSet::new();
            if let Some(file) = outfile.as_mut()
                && append
            {
                let mut scratch = Vec::new();
                file.read_to_end(&mut scratch)?;
                get_citekeys(SourceFileType::Bib, &scratch, &mut skipped_keys);
            }

            // Collect all entries which are not null, excluding those which should be skipped
            let cfg = config::load(&config_path, missing_ok)?;
            let not_skipped_keys = citation_keys
                .into_iter()
                .filter(|k| !skipped_keys.contains(k));

            let valid_entries = if cli.read_only {
                retrieve_entries_read_only(
                    not_skipped_keys,
                    &mut record_db,
                    retrieve_only,
                    ignore_null,
                    &cfg,
                )
            } else {
                retrieve_and_validate_entries(
                    not_skipped_keys,
                    &mut record_db,
                    client,
                    retrieve_only,
                    ignore_null,
                    &cfg,
                )
            };

            if !retrieve_only {
                output_entries(outfile, append, valid_entries)?;
            }
        }
        Command::Import {
            targets,
            local,
            determine_key,
            retrieve,
            retrieve_only,
            no_alias,
            replace_colons,
            log_failures,
            prefer_current,
            prefer_incoming,
        } => {
            let replace_colons = match replace_colons {
                Some(subst) => match EntryKey::try_new(subst) {
                    Ok(new) => Some(new),
                    Err(err) => bail!("Argument to `--replace-colons` is invalid: {err}"),
                },
                None => None,
            };

            let import_config = self::import::ImportConfig {
                update_mode: UpdateMode::from_flags(
                    cli.no_interactive,
                    prefer_current,
                    prefer_incoming,
                ),
                import_mode: ImportMode::from_flags(local, determine_key, retrieve, retrieve_only),
                no_alias,
                no_interactive: cli.no_interactive,
                replace_colons,
                log_failures,
            };

            debug!("Using import configuration: {import_config:?}");
            let cfg = config::load(&config_path, missing_ok)?;

            let mut scratch = Vec::new();

            for bibfile in targets {
                scratch.clear();
                match File::open(&bibfile).and_then(|mut file| file.read_to_end(&mut scratch)) {
                    Ok(_) => {
                        import::from_buffer(
                            &scratch,
                            &import_config,
                            &mut record_db,
                            client,
                            &cfg,
                            bibfile.display(),
                        )?;
                    }
                    Err(err) => error!(
                        "Failed to read contents of file '{}': {err}",
                        bibfile.display()
                    ),
                }
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
                RecordIdState::Unknown(unknown) => {
                    let maybe_normalized = unknown.combine_and_commit()?;
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

            let (row, raw_data) = if let Some(old_id) = rename_from {
                // Allowing for arbitrary `old_id` without validation and trimming
                // so that local ids that were valid in an older version can be renamed.
                // SAFETY: This is safe as a colon is present in the `full_id`.
                let old_remote_id = RemoteId::from_string_unchecked("local:".to_owned() + &old_id);
                match record_db
                    .state_from_remote_id(&old_remote_id)?
                    .delete_null()?
                {
                    ExistsOrUnknown::Existent(row) => {
                        if !row.change_canonical_id(&remote_id)? {
                            bail!("Local record '{remote_id}' already exists")
                        }

                        if let Ok(old_alias) = Alias::from_str(&old_id) {
                            row.delete_alias_if_associated(&old_alias)?;
                        }

                        let raw_record_data = row.get_data()?.data;
                        (row, raw_record_data)
                    }
                    ExistsOrUnknown::Unknown(missing) => {
                        missing.commit()?;
                        bail!("Local record '{old_remote_id}' does not exist");
                    }
                }
            } else {
                match record_db.state_from_remote_id(&remote_id)?.delete_null()? {
                    ExistsOrUnknown::Existent(row) => {
                        if from.is_some() {
                            row.commit()?;
                            bail!("Local record '{remote_id}' already exists")
                        } else {
                            let raw_record_data = row.get_data()?.data;
                            (row, raw_record_data)
                        }
                    }
                    ExistsOrUnknown::Unknown(missing) => {
                        let data = data_from_path_or_default(from.as_ref())?;
                        let raw_record_data = RawRecordData::from_entry_data(&data);
                        let row = missing.insert(&raw_record_data, &remote_id)?;
                        (row, raw_record_data)
                    }
                }
            };

            let edit_key_candidate = if no_alias {
                remote_id.name()
            } else {
                info!("Creating alias '{alias}' for '{remote_id}'");
                match row.ensure_alias(&alias)? {
                    Some(other_remote_id) => {
                        warn!(
                            "Alias '{alias}' already exists and refers to '{other_remote_id}'. '{remote_id}' will be a different record."
                        );
                        remote_id.name()
                    }
                    _ => alias.name(),
                }
            };

            if !cli.no_interactive {
                edit_record_and_update(
                    &row,
                    Entry {
                        key: EntryKey::try_new(edit_key_candidate.into())
                            .unwrap_or_else(|_| EntryKey::placeholder()),
                        record_data: RecordData::from_entry_data(&raw_data),
                    },
                    false,
                    &remote_id,
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
            let (existing, row) = get_record_row(&mut record_db, into, client, &cfg)?
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
            let mut existing_record = RecordData::from_entry_data(&existing.data);
            merge_record_data(
                UpdateMode::from_flags(cli.no_interactive, prefer_current, prefer_incoming),
                &mut existing_record,
                new_data.iter().map(|row_data| &row_data.data),
                &existing.key,
            )?;

            // update the row data with the modified data
            row.save_to_changelog()?;
            row.update(&RawRecordData::from_entry_data(&existing_record))?;
            row.commit()?;
        }
        Command::Path {
            citation_key,
            mkdir,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;
            // Extend with the filename.
            let (record, row) = get_record_row(&mut record_db, citation_key, client, &cfg)?
                .exists_or_commit_null("Cannot show directory for")?;
            row.commit()?;
            let mut target = get_attachment_dir(&data_dir, cli.attachments_dir, &record.canonical)?;

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
            append,
            skip,
            skip_from,
            skip_file_type,
            retrieve_only,
            ignore_null,
            print_keys,
        } => {
            let mut outfile = init_outfile(out, append)?;
            let mut scratch = Vec::new();

            // initialize skipped keys with:
            // - explicitly passed keys
            // - keys from the provided files
            // - any keys in the output bibfile, if appending
            let mut skipped_keys: HashSet<RecordId> = HashSet::new();
            skipped_keys.extend(skip);
            for skip_path in skip_from {
                source::get_citekeys_from_file(
                    skip_path,
                    skip_file_type,
                    &mut skipped_keys,
                    &mut scratch,
                    "--skip-file-type",
                )?;
            }
            if let Some(file) = outfile.as_mut()
                && append
            {
                // must call `rewind` here since the `append` open option may set the 'read'
                // cursor position to the end of the file, depending on the platform
                file.rewind()?;
                // read the file into the buffer
                file.read_to_end(&mut scratch)?;
                get_citekeys(SourceFileType::Bib, &scratch, &mut skipped_keys);
            }

            if print_keys {
                // only print the keys which were found
                let mut all_citekeys: BTreeSet<RecordId> = BTreeSet::new();

                for path in paths {
                    source::get_citekeys_from_file_filter(
                        path,
                        file_type,
                        &mut all_citekeys,
                        &mut scratch,
                        "--file-type",
                        |record_id| !skipped_keys.contains(record_id),
                    )?;
                }
                output_keys(all_citekeys.iter())?;
            } else {
                // read citation keys from all of the paths, excluding those which are present in
                // 'skipped_keys'
                //
                // The citation keys do not need to be sorted since sorting
                // happens in the `validate_and_retrieve` function.
                let mut all_citekeys: HashSet<RecordId> = HashSet::new();

                for path in paths {
                    source::get_citekeys_from_file_filter(
                        path,
                        file_type,
                        &mut all_citekeys,
                        &mut scratch,
                        "--file-type",
                        |record_id| !skipped_keys.contains(record_id),
                    )?;
                }

                // retrieve all of the entries
                let cfg = config::load(&config_path, missing_ok)?;
                let keys = all_citekeys.into_iter();
                let valid_entries = if cli.read_only {
                    retrieve_entries_read_only(
                        keys,
                        &mut record_db,
                        retrieve_only,
                        ignore_null,
                        &cfg,
                    )
                } else {
                    retrieve_and_validate_entries(
                        keys,
                        &mut record_db,
                        client,
                        retrieve_only,
                        ignore_null,
                        &cfg,
                    )
                };

                if !retrieve_only {
                    output_entries(outfile, append, valid_entries)?;
                }
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
                    let new_raw_data = match data_from_path_or_remote(from, canonical, client) {
                        Ok((data, _)) => data,
                        Err(err) => {
                            row.commit()?;
                            bail!(err);
                        }
                    };
                    let mut existing_record = RecordData::from_entry_data(&existing_raw_data);
                    merge_record_data(
                        UpdateMode::from_flags(cli.no_interactive, prefer_current, prefer_incoming),
                        &mut existing_record,
                        once(&new_raw_data),
                        &citation_key,
                    )?;
                    row.save_to_changelog()?;
                    row.update(&RawRecordData::from_entry_data(&existing_record))?;
                    row.commit()?;
                }
                RecordIdState::NullRemoteId(mapped_remote_id, null_row) => {
                    match data_from_path_or_remote(from, mapped_remote_id.mapped, client) {
                        Ok((data, canonical)) => {
                            info!("Existing row was null; inserting new data.");
                            let row = null_row.delete()?.insert_entry_data(&data, &canonical)?;
                            row.commit()?;
                        }
                        Err(err) => {
                            null_row.commit()?;
                            bail!(err);
                        }
                    };
                }
                RecordIdState::Unknown(unknown) => {
                    let maybe_normalized = unknown.combine_and_commit()?;
                    error!("Record does not exist in database: {maybe_normalized}");
                    if !maybe_normalized.mapped.is_local() {
                        suggest!("Use `autobib get` to retrieve record");
                    }
                }
                RecordIdState::UndefinedAlias(alias) => {
                    bail!("Undefined alias: '{alias}'");
                }
                RecordIdState::InvalidRemoteId(err) => bail!("{err}"),
            }
        }
        Command::Util { util_command } => match util_command {
            UtilCommand::Check { fix } => {
                info!(
                    "Validating record binary data and consistency, and checking for dangling records."
                );
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
            UtilCommand::Optimize => {
                info!("Optimizing database.");
                record_db.vacuum()?;
            }
            UtilCommand::Evict { max_age } => match max_age {
                Some(seconds) => {
                    record_db.evict_cache_max_age(seconds)?;
                }
                None => {
                    record_db.evict_cache()?;
                }
            },
            UtilCommand::List { canonical } => {
                record_db.map_citation_keys(canonical, |key_str| {
                    println!("{key_str}");
                })?;
            }
        },
    };

    Ok(())
}
