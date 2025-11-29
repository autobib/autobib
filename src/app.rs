mod cli;
mod delete;
mod edit;
mod hist;
mod import;
mod info;
mod log;
mod path;
mod picker;
mod retrieve;
mod source;
mod update;
mod write;

use std::{
    collections::{BTreeSet, HashSet},
    fs::{File, OpenOptions, create_dir_all, exists},
    io::{IsTerminal, Read, Seek, Write, copy},
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{Result, bail};
use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};

use crate::{
    app::{
        cli::{HistCommand, PruneCommand},
        log::print_log,
    },
    cite_search::{SourceFileType, get_citekeys},
    config,
    db::{
        DeleteAliasResult, RecordDatabase, RenameAliasResult,
        state::{
            DisambiguatedRecordRow, ExistsOrUnknown, RecordIdState, RecordRowMoveResult,
            RemoteIdState, SetActiveError,
        },
        user_version,
    },
    entry::{Entry, EntryEditCommand, EntryKey, MutableEntryData, RawEntryData},
    error::AliasErrorKind,
    format::Template,
    http::{BodyBytes, Client},
    logger::{Level, debug, error, info, max_level, suggest, warn},
    normalize::{Normalization, Normalize},
    output::{owriteln, stdout_lock_wrap},
    record::{Alias, Record, RecordId, RemoteId, get_record_row},
    term::Editor,
};

use self::{
    cli::{AliasCommand, FindMode, InfoReportType, OnConflict, UtilCommand},
    delete::{hard_delete, soft_delete},
    edit::{create_alias_if_valid, insert, merge_record_data},
    path::{data_from_key, data_from_path, get_attachment_dir, get_attachment_root},
    picker::{choose_attachment, choose_attachment_path, choose_canonical_id},
    retrieve::{retrieve_and_validate_entries, retrieve_entries_read_only},
    update::update,
    write::{init_outfile, output_entries, output_keys},
};

pub use self::cli::{Cli, Command};

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

    info!("Interactive: {}", !cli.no_interactive);
    info!("Read-only: {}", cli.read_only);

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
                        bail!("Alias already exists: '{new}'");
                    }
                }
            }
            AliasCommand::Reset { alias, target } => {
                info!("Updating alias '{alias}' to point to '{target}'");
                let cfg = config::load(&config_path, missing_ok)?;
                let (_, row) = get_record_row(&mut record_db, target, client, &cfg)?
                    .exists_or_commit_null("Cannot create alias for")?;
                if !row.update_alias(&alias)? {
                    error!("Alias does not exist!");
                    suggest!("Use `autobib alias add` to insert a new alias.");
                }
                row.commit()?;
            }
        },
        Command::Attach {
            identifier,
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
            let (record, row) = get_record_row(&mut record_db, identifier, client, &cfg)?
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
                    let mut target_file = opts.open(&target)?;
                    if let Err(e) = copy(&mut body.as_reader(), &mut target_file) {
                        error!("{e}");
                        // check if there is a file at the target location; if there is one, it
                        // could be the partially downloaded file
                        match exists(&target) {
                            Ok(false) => {}
                            _ => {
                                warn!(
                                    "The file may have partially downloaded at the below path:\n
                                {}",
                                    target.display()
                                );
                            }
                        }
                    }
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
            config::write_default(stdout_lock_wrap())?;
        }
        Command::Delete {
            identifiers,
            replace,
            hard,
            update_aliases,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;
            if hard {
                for key in identifiers {
                    hard_delete(key, &mut record_db, &cfg)?;
                }
            } else {
                let replacement_canonical_id = match replace {
                    None => None,
                    Some(replacement) => Some(
                        match record_db.state_from_record_id(replacement, &cfg.alias_transform)? {
                            RecordIdState::Entry(_, data, state) => {
                                state.commit()?;
                                data.canonical
                            }
                            RecordIdState::Deleted(_, data, state) => {
                                state.commit()?;
                                data.canonical
                            }
                            RecordIdState::Void(_, data, state) => {
                                state.commit()?;
                                data.canonical
                            }
                            RecordIdState::NullRemoteId(mapped_key, state) => {
                                state.commit()?;
                                bail!(
                                    "Invalid replacement key {mapped_key} corresponds to a null record."
                                );
                            }
                            RecordIdState::Unknown(unknown) => {
                                let maybe_normalized = unknown.combine_and_commit()?;
                                bail!(
                                    "Invalid replacement key {maybe_normalized}: does not exist in the database."
                                );
                            }
                            RecordIdState::UndefinedAlias(alias) => {
                                bail!("Invalid replacement key: alias '{alias}' is undefined")
                            }
                            RecordIdState::InvalidRemoteId(record_error) => bail!("{record_error}"),
                        },
                    ),
                };

                for key in identifiers {
                    soft_delete(
                        key,
                        &replacement_canonical_id,
                        &mut record_db,
                        &cfg,
                        update_aliases,
                    )?;
                }
            }
        }
        Command::Edit {
            identifiers,
            normalize_whitespace,
            set_eprint,
            strip_journal_series,
            update_entry_type,
            set_field,
            delete_field,
            touch,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;
            let nl = Normalization {
                normalize_whitespace,
                set_eprint,
                strip_journal_series,
            };

            let edit_cmd = EntryEditCommand {
                update_entry_type,
                set_field,
                delete_field,
            };

            let no_non_interactive_cmd = nl.is_identity() && edit_cmd.is_identity() && !touch;

            for key in identifiers {
                let (Record { key, data, .. }, row) =
                    get_record_row(&mut record_db, key, client, &cfg)?
                        .exists_or_commit_null("Cannot edit")?;

                match (cli.no_interactive, no_non_interactive_cmd) {
                    (true, true) => {
                        warn!("Terminal is non-interactive and no edit action specified!");
                        row.commit()?;
                    }
                    (_, false) => {
                        // non-interactive command is requested, so we perform it without prompting
                        let mut editable_data = MutableEntryData::from_entry_data(&data);
                        let mut changed = touch;

                        changed |= editable_data.normalize(&nl);
                        changed |= editable_data.edit(&edit_cmd);

                        if changed {
                            row.modify(&RawEntryData::from_entry_data(&editable_data))?
                                .commit()?;
                        } else {
                            row.commit()?;
                        }
                    }
                    (false, true) => {
                        // only perform normalization
                        let record_data = MutableEntryData::from_entry_data(&data);
                        let entry = Entry {
                            key: EntryKey::try_new(key).unwrap_or_else(|_| EntryKey::placeholder()),
                            record_data,
                        };

                        if let Some(Entry { key, record_data }) =
                            Editor::new_bibtex().edit(&entry)?
                        {
                            let new_row =
                                row.modify(&RawEntryData::from_entry_data(&record_data))?;
                            if key.as_ref() != entry.key.as_ref() {
                                create_alias_if_valid(key.as_ref(), &new_row)?;
                            }
                            new_row.commit()?;
                        } else {
                            // we return an error here, since this was an interactive edit
                            row.commit()?;
                            error!("Record data unchanged");
                        }
                    }
                };
            }
        }
        Command::Find {
            template: format,
            strict,
            mode: find_mode,
        } => {
            if cli.no_interactive {
                bail!("`autobib find` cannot run in non-interactive mode");
            }

            let cfg = config::load(&config_path, missing_ok)?;

            // read template, or load from config / use default
            let template = match format {
                Some(t) => t,
                None => match Template::compile(&cfg.find.default_template) {
                    Ok(t) => t,
                    Err(err) => {
                        bail!("Syntax error in `find.default_template` configuration value: {err}");
                    }
                },
            };

            match find_mode {
                FindMode::Attachments => {
                    let mut picker = choose_attachment_path(
                        record_db,
                        template,
                        strict,
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
                                        owriteln!("{}", dir_entry.path().display())?;
                                    }
                                    None => error!("No attachment selected."),
                                }
                            } else {
                                owriteln!("{}", data.attachments.first().path().display())?;
                            };
                        }
                        None => error!("No record selected."),
                    }
                }
                FindMode::CanonicalId => {
                    let (mut picker, handle) = choose_canonical_id(record_db, template, strict);
                    match picker.pick()? {
                        Some(row_data) => {
                            let cfg = config::load(&config_path, missing_ok)?;
                            if !cfg.preferred_providers.is_empty() {
                                // get a key from the preferred provider if possible
                                let mut record_db =
                                    handle.join().expect("Thread should not have panicked")?;
                                match record_db.state_from_remote_id(&row_data.canonical)? {
                                    RemoteIdState::Entry(_, row) => {
                                        // try to find a referencing key with the expected provider
                                        let referencing_ids = row.referencing_remote_ids()?;
                                        for provider in cfg.preferred_providers {
                                            if let Some(remote_id) = referencing_ids
                                                .iter()
                                                .find(|id| id.provider() == provider)
                                            {
                                                owriteln!("{remote_id}")?;
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
                            owriteln!("{}", row_data.canonical)?;
                        }
                        None => error!("No item selected."),
                    }
                }
            }
        }
        Command::Get {
            identifiers,
            out,
            append,
            retrieve_only,
            ignore_null,
        } => {
            let mut outfile = init_outfile(out, append)?;

            // Initialize the skipped keys to contain keys already present in the outfile (if
            // appending)
            let mut skipped_ids: HashSet<RecordId> = HashSet::new();
            if let Some(file) = outfile.as_mut()
                && append
            {
                let mut scratch = Vec::new();
                file.read_to_end(&mut scratch)?;
                get_citekeys(SourceFileType::Bib, &scratch, &mut skipped_ids);
            }

            // Collect all entries which are not null, excluding those which should be skipped
            let cfg = config::load(&config_path, missing_ok)?;
            let not_skipped_ids = identifiers.into_iter().filter(|k| !skipped_ids.contains(k));

            let valid_entries = if cli.read_only {
                retrieve_entries_read_only(
                    not_skipped_ids,
                    &mut record_db,
                    retrieve_only,
                    ignore_null,
                    &cfg,
                )
            } else {
                retrieve_and_validate_entries(
                    not_skipped_ids,
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
        Command::Hist { hist_command } => match hist_command {
            HistCommand::Prune { prune_command } => {
                let snapshot = record_db.snapshot()?;
                match prune_command {
                    PruneCommand::All => snapshot.prune_all()?,
                    PruneCommand::Deleted => snapshot.prune_deleted()?,
                    PruneCommand::Outdated { retain } => match retain {
                        0 => snapshot.prune_outdated()?,
                        idx => snapshot.prune_outdated_keep(idx)?,
                    },
                }
                snapshot.commit()?;
            }
            HistCommand::Redo {
                identifier,
                index,
                revive,
            } => {
                let index = index.unwrap_or(-1);
                let cfg = config::load(&config_path, missing_ok)?;
                match record_db
                    .state_from_record_id(identifier, &cfg.alias_transform)?
                    .require_record()?
                {
                    Some((_, DisambiguatedRecordRow::Entry(_, state))) => {
                        if revive {
                            error!(
                                "Attempted to redo from a deleted state, but the record currently exists"
                            );
                        } else {
                            hist::handle_redo_result(state.redo(index)?)?;
                        }
                    }
                    Some((_, DisambiguatedRecordRow::Deleted(_, state))) => {
                        if revive {
                            hist::handle_redo_result(state.redo_deletion(index)?)?;
                        } else if state.current()?.has_children()? {
                            error!("Cannot redo beyond a deletion marker");
                            suggest!(
                                "Redo from a deleted state using `autobib hist redo --revive`"
                            );
                            suggest!("Insert new data with `autobib hist revive`");
                            state.commit()?;
                        } else {
                            error!("No changes to redo");
                            suggest!("Insert new data with `autobib hist revive`");
                            state.commit()?;
                        }
                    }
                    Some((_, DisambiguatedRecordRow::Void(_, state))) => {
                        if revive {
                            hist::handle_redo_result(state.redo_deletion(index)?)?;
                        } else if state.current()?.has_children()? {
                            error!("Cannot redo from the voided state.");
                            suggest!(
                                "Redo from a voided state using `autobib hist redo --revive`, or insert new data"
                            );
                            suggest!("Insert new data with `autobib hist revive`");
                            state.commit()?;
                        } else {
                            error!("No changes to redo");
                            suggest!("Insert new data with `autobib hist revive`");
                            state.commit()?;
                        }
                    }
                    None => {}
                }
            }
            HistCommand::Reset {
                identifier,
                rev,
                before,
            } => {
                let cfg = config::load(&config_path, missing_ok)?;
                if let Some((_, disambiguated)) = record_db
                    .state_from_record_id(identifier, &cfg.alias_transform)?
                    .require_record()?
                {
                    let (_, state) = disambiguated.forget();

                    if let Some(revision) = rev {
                        match state.set_active(revision)? {
                            RecordRowMoveResult::Updated(state) => {
                                if max_level() >= Level::Warn {
                                    let version = state.current()?;
                                    let mut stdout = stdout_lock_wrap();
                                    let styled = stdout.supports_styled_output();
                                    writeln!(&mut stdout, "{}", version.display(styled))?;
                                }
                                state.commit()?;
                            }
                            RecordRowMoveResult::Unchanged(state, err) => {
                                state.commit()?;
                                match err {
                                    SetActiveError::RowIdUndefined => {
                                        error!("Revision does not exist in the 'Records' table");
                                    }
                                    SetActiveError::DifferentCanonical(remote_id) => {
                                        error!(
                                            "Revision exists, but it corresponds to a different record with canonical identifier '{remote_id}'"
                                        );
                                    }
                                }
                            }
                        }
                    } else if let Some(dt) = before {
                        let state = state.rewind(dt)?;
                        if max_level() >= Level::Warn {
                            let version = state.current()?;
                            let mut stdout = stdout_lock_wrap();
                            let styled = stdout.supports_styled_output();
                            writeln!(&mut stdout, "{}", version.display(styled))?;
                        }
                        state.commit()?;
                    }
                }
            }
            HistCommand::Revive {
                identifier,
                from_bibtex,
                with_entry_type,
                with_field,
            } => {
                let cfg = config::load(&config_path, missing_ok)?;
                let edit_cmd = EntryEditCommand {
                    update_entry_type: with_entry_type,
                    set_field: with_field,
                    delete_field: Vec::new(),
                };
                match record_db
                    .state_from_record_id(identifier, &cfg.alias_transform)?
                    .require_record()?
                {
                    Some((_, DisambiguatedRecordRow::Entry(_, state))) => {
                        state.commit()?;
                        bail!("Record already exists!")
                    }
                    Some((_, DisambiguatedRecordRow::Deleted(data, state))) => {
                        insert(
                            state,
                            from_bibtex,
                            &data.canonical,
                            cli.no_interactive,
                            &cfg.on_insert,
                            &edit_cmd,
                        )?;
                    }
                    Some((_, DisambiguatedRecordRow::Void(data, state))) => {
                        insert(
                            state,
                            from_bibtex,
                            &data.canonical,
                            cli.no_interactive,
                            &cfg.on_insert,
                            &edit_cmd,
                        )?;
                    }
                    None => {}
                }
            }
            HistCommand::RewindAll { before } => {
                let snapshot = record_db.snapshot()?;
                snapshot.rewind_all(before)?;
                snapshot.commit()?;
            }
            HistCommand::Show { limit } => {
                let snapshot = record_db.snapshot()?;
                let mut stdout = stdout_lock_wrap();
                let styled = stdout.supports_styled_output();
                snapshot.map_history(limit, |record_row, rev_id| {
                    let disp = crate::db::state::tree::RecordRowDisplay::from_borrowed_row(
                        record_row, rev_id, styled,
                    );
                    writeln!(&mut stdout, "{disp}\n")
                })?;
                snapshot.commit()?;
            }
            HistCommand::Undo { identifier, delete } => {
                let cfg = config::load(&config_path, missing_ok)?;
                match record_db
                    .state_from_record_id(identifier, &cfg.alias_transform)?
                    .require_record()?
                {
                    Some((_, DisambiguatedRecordRow::Entry(_, state))) => {
                        if delete {
                            hist::handle_undo_result(state.undo_delete()?)?;
                        } else {
                            hist::handle_undo_result(state.undo()?)?;
                        };
                    }
                    Some((_, DisambiguatedRecordRow::Deleted(_, state))) => {
                        if delete {
                            hist::handle_undo_result(state.undo_delete()?)?;
                        } else {
                            hist::handle_undo_result(state.undo()?)?;
                        };
                    }
                    Some((_, DisambiguatedRecordRow::Void(_, _))) => {
                        error!("Nothing to undo!");
                    }
                    None => {}
                }
            }
            HistCommand::Void { identifier } => {
                let cfg = config::load(&config_path, missing_ok)?;
                match record_db
                    .state_from_record_id(identifier, &cfg.alias_transform)?
                    .require_record()?
                {
                    Some((_, DisambiguatedRecordRow::Entry(_, state))) => {
                        state.void()?.commit()?;
                    }
                    Some((_, DisambiguatedRecordRow::Deleted(_, state))) => {
                        state.void()?.commit()?;
                    }
                    Some((_, DisambiguatedRecordRow::Void(_, state))) => {
                        state.commit()?;
                        error!("Record is already void");
                    }
                    None => {}
                }
            }
        },
        Command::Import {
            targets,
            mode,
            no_alias,
            replace_colons,
            log_failures,
            on_conflict,
        } => {
            let replace_colons = match replace_colons {
                Some(subst) => match EntryKey::try_new(subst) {
                    Ok(new) => Some(new),
                    Err(err) => bail!("Argument to `--replace-colons` is invalid: {err}"),
                },
                None => None,
            };

            let import_config = self::import::ImportConfig {
                on_conflict,
                import_mode: mode,
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
        Command::Info { identifier, report } => {
            let cfg = config::load(&config_path, missing_ok)?;
            match record_db.state_from_record_id(identifier, &cfg.alias_transform)? {
                RecordIdState::Entry(key, data, state) => {
                    info::database_report(key, data, state, report, |_, stdout| {
                        writeln!(stdout, "Record with data")
                    })?;
                }
                RecordIdState::Deleted(key, data, state) => {
                    info::database_report(key, data, state, report, |data, stdout| {
                        if let Some(repl) = data {
                            writeln!(stdout, "Deleted and replaced by reference: {repl}")
                        } else {
                            writeln!(stdout, "Deleted record")
                        }
                    })?;
                }
                RecordIdState::Void(key, data, state) => {
                    info::database_report(key, data, state, report, |_, stdout| {
                        writeln!(stdout, "Voided record")
                    })?;
                }
                RecordIdState::NullRemoteId(remote_id, null_row) => match report {
                    InfoReportType::All => {
                        owriteln!("Null record: {remote_id}")?;
                        let null_row_data = null_row.get_data()?;
                        owriteln!("Last attempted: {}", null_row_data.attempted)?;
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
                    InfoReportType::Revision => {
                        bail!("No revision for null record '{remote_id}'");
                    }
                    InfoReportType::Modified => {
                        owriteln!("{}", null_row.get_null_attempted()?)?;
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
            from_bibtex,
            with_entry_type,
            with_field,
        } => {
            // check if the provided identifier is a valid alias
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
            let edit_cmd = EntryEditCommand {
                update_entry_type: with_entry_type,
                set_field: with_field,
                delete_field: Vec::new(),
            };

            // insert the data
            match record_db.state_from_remote_id(&remote_id)?.delete_null()? {
                ExistsOrUnknown::Entry(_, state) => {
                    state.commit()?;
                    bail!("Local record '{remote_id}' already exists")
                }
                ExistsOrUnknown::Deleted(_, state) => {
                    state.commit()?;
                    error!("Local record '{remote_id}' was soft-deleted");
                    suggest!(
                        "Use `autobib hist undo` to recover past data or `autobib hist revive` to insert new data"
                    );
                }
                ExistsOrUnknown::Void(_, void) => {
                    let cfg = config::load(&config_path, missing_ok)?;
                    insert(
                        void,
                        from_bibtex,
                        &remote_id,
                        cli.no_interactive,
                        &cfg.on_insert,
                        &edit_cmd,
                    )?;
                }
                ExistsOrUnknown::Unknown(missing) => {
                    let cfg = config::load(&config_path, missing_ok)?;
                    insert(
                        missing,
                        from_bibtex,
                        &remote_id,
                        cli.no_interactive,
                        &cfg.on_insert,
                        &edit_cmd,
                    )?;
                }
            };
        }
        Command::Log {
            identifier,
            tree,
            all,
            reverse,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;
            if let Some((_, entry_or_deleted)) = record_db
                .state_from_record_id(identifier, &cfg.alias_transform)?
                .require_record()?
            {
                let (_, state) = entry_or_deleted.forget();
                print_log(cli.no_interactive, &state, tree, all, reverse, false)?;
                state.commit()?;
            }
        }
        Command::Path { identifier, mkdir } => {
            let cfg = config::load(&config_path, missing_ok)?;
            // Extend with the filename.
            let (record, row) = get_record_row(&mut record_db, identifier, client, &cfg)?
                .exists_or_commit_null("Cannot show directory for")?;
            row.commit()?;
            let mut target = get_attachment_dir(&data_dir, cli.attachments_dir, &record.canonical)?;

            if mkdir {
                create_dir_all(&target)?;
            }

            // This appends a `/` or `\` when printing, as platform appropriate, to be clear to the
            // user that this is a directory
            target.push("");

            owriteln!("{}", target.display())?;
        }
        Command::Source {
            paths,
            file_type,
            out,
            stdin,
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

            if paths.is_empty() && stdin.is_none() && !std::io::stdin().is_terminal() {
                warn!("Text written to standard input is being ignored");
                suggest!("Use `--stdin FILE_TYPE` to search for identifiers in standard input.");
            }

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

                if let Some(stdin_file_type) = stdin {
                    source::get_citekeys_from_stdin(
                        stdin_file_type,
                        &mut all_citekeys,
                        &mut scratch,
                        |record_id| !skipped_keys.contains(record_id),
                    )?;
                }

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
                // read identifiers from all of the paths, excluding those which are present in
                // 'skipped_keys'
                //
                // The ids do not need to be sorted since sorting
                // happens in the `validate_and_retrieve` function.
                let mut all_citekeys: HashSet<RecordId> = HashSet::new();

                if let Some(stdin_file_type) = stdin {
                    source::get_citekeys_from_stdin(
                        stdin_file_type,
                        &mut all_citekeys,
                        &mut scratch,
                        |record_id| !skipped_keys.contains(record_id),
                    )?;
                }

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
            identifier,
            from_bibtex,
            from_record: from_key,
            on_conflict,
            revive,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;

            let provided_data = if let Some(record_id) = from_key {
                Some(data_from_key(&mut record_db, record_id, &cfg)?)
            } else if let Some(path) = from_bibtex {
                Some(data_from_path(path)?)
            } else {
                None
            };

            update(
                on_conflict,
                record_db.state_from_record_id(identifier, &cfg.alias_transform)?,
                provided_data,
                client,
                &cfg.on_insert,
                revive,
            )?;
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
                let mut lock = stdout_lock_wrap();
                let snapshot = record_db.snapshot()?;
                snapshot.map_identifiers(canonical, |key_str| writeln!(lock, "{key_str}"))?;
                snapshot.commit()?;
            }
        },
    };

    Ok(())
}
