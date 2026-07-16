mod attach;
mod cli;
mod delete;
mod edit;
mod find;
mod get;
mod hist;
mod import;
mod info;
mod log;
mod picker;
mod replace;
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
        attach::cleanup_empty_attachment_dirs,
        cli::{GcCommand, HistCommand, IdTarget, PruneCommand},
        log::print_log,
        retrieve::{sync_entries, sync_entries_read_only},
    },
    cite_search::{SourceFileType, get_citekeys},
    config,
    db::{
        DeleteAliasResult, RecordDatabase, RenameAliasResult,
        state::{
            DisambiguatedRecordState, ExistsOrUnknown, RecordIdState, RecordRowDisplay,
            RecordRowMoveResult, SetActiveError,
        },
        user_version,
    },
    entry::{Entry, EntryEditCommand, EntryKey, MutableEntryData, RawEntryData},
    error::AliasErrorKind,
    format::Template,
    http::{BodyBytes, Client},
    logger::{LogDisplay, debug, error, info, suggest, warn},
    normalize::{Normalization, Normalize},
    output::{owriteln, stdout_lock_wrap},
    path_hash::AttachmentRoot,
    provider::{RemoteIdCandidate, determine_key_from_data},
    record::{Alias, Record, RecordId, RemoteId, get_record_row, get_record_row_tx},
    term::Editor,
};

use self::{
    attach::{
        get_attachment_root, get_attachment_root_path, get_existing_attachment_root,
        migrate_attachments,
    },
    cli::{AliasCommand, FindMode, InfoReportType, OnConflict, UtilCommand},
    delete::{hard_delete, soft_delete},
    edit::{create_alias_if_valid, insert, merge_record_data},
    find::output_find_selection,
    import::ImportConfig,
    picker::{choose_attachment, choose_attachment_path, choose_canonical_id},
    retrieve::{retrieve_entries, retrieve_entries_read_only},
    update::{data_from_key, data_from_path, data_from_rev, update},
    write::{init_outfile, output_entries_bibtex, output_entries_json, output_keys},
};

pub use self::cli::{Cli, Command};

fn handle_deprecation(cmd: Command) -> Command {
    match cmd {
        Command::Util { util_command } => match util_command {
            UtilCommand::List { canonical, deleted } => {
                warn!("`autobib util list` is deprecated; use `autobib list` instead.");
                Command::List {
                    matching: String::from("*"),
                    canonical,
                    deleted,
                    template: None,
                    strict: false,
                    sep: String::from("\n"),
                }
            }
            UtilCommand::Optimize => {
                warn!(
                    "`autobib util optimize` is deprecated; use `autobib clean database --compact`"
                );
                Command::Clean {
                    gc_command: GcCommand::Database {
                        compact: true,
                        evict: None,
                        evict_all: false,
                    },
                }
            }
            UtilCommand::Evict { max_age } => {
                warn!("`autobib util evict` is deprecated; use `autobib clean database`");
                Command::Clean {
                    gc_command: GcCommand::Database {
                        compact: false,
                        evict: max_age,
                        evict_all: max_age.is_none(),
                    },
                }
            }
            util_command => Command::Util { util_command },
        },
        cmd => cmd,
    }
}

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

    let command = handle_deprecation(cli.command);

    // Run the cli
    match command {
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
            AliasCommand::Reassign { alias, target } => {
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
            let root = get_attachment_root(&data_dir, cli.attachments_dir)?;
            let mut target = root.attachment_dir(&record.row.canonical);

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
        Command::Backup { into } => {
            if let Some(par) = into.parent() {
                std::fs::create_dir_all(par)?;
            }
            record_db.vacuum_into(&into)?;
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
            hard,
            delete_aliases,
            no_attachment_warning,
        } => {
            fn do_delete<F>(
                identifiers: Vec<RecordId>,
                check_attachments: &Option<AttachmentRoot<false>>,
                mut delete_cb: F,
            ) -> Result<(), anyhow::Error>
            where
                F: FnMut(RecordId) -> Result<Option<RemoteId>, rusqlite::Error>,
            {
                for key in identifiers {
                    if let Some(canonical) = delete_cb(key)?
                        && let Some(at_root) = check_attachments.as_ref()
                        && at_root.exists(&canonical)?
                    {
                        warn!(
                            "Deleted record has attachment directory: {}",
                            at_root.attachment_dir(&canonical).display()
                        );
                    }
                }
                Ok(())
            }

            let cfg = config::load(&config_path, missing_ok)?;
            let attachment_root = if no_attachment_warning {
                None
            } else {
                get_existing_attachment_root(&data_dir, cli.attachments_dir)?
            };

            if hard {
                do_delete(identifiers, &attachment_root, |key| {
                    hard_delete(key, &mut record_db, &cfg)
                })?;
            } else {
                do_delete(identifiers, &attachment_root, |key| {
                    soft_delete(key, &None, &mut record_db, &cfg, delete_aliases)
                })?;
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

            let no_non_interactive_cmd = nl.is_identity() && edit_cmd.is_identity();

            for key in identifiers {
                let (record, state) = get_record_row(&mut record_db, key, client, &cfg)?
                    .exists_or_commit_null("Cannot edit")?;

                match (cli.no_interactive, no_non_interactive_cmd) {
                    (true, true) => {
                        warn!("Terminal is non-interactive and no edit action specified!");
                        state.commit()?;
                    }
                    (_, false) => {
                        // non-interactive command is requested, so we perform it without prompting
                        let mut editable_data = MutableEntryData::from_entry_data(&record.row.data);

                        let changed = editable_data.normalize(&nl) || editable_data.edit(&edit_cmd);

                        if changed {
                            state
                                .modify(&RawEntryData::from_entry_data(&editable_data))?
                                .state
                                .commit()?;
                        } else {
                            state.commit()?;
                        }
                    }
                    (false, true) => {
                        // only perform normalization
                        let record_data = MutableEntryData::from_entry_data(&record.row.data);
                        let entry = Entry {
                            key: EntryKey::try_new(record.key)
                                .unwrap_or_else(|_| EntryKey::placeholder()),
                            record_data,
                        };

                        if let Some(Entry { key, record_data }) =
                            Editor::new_bibtex().edit(&entry)?
                        {
                            let new_row = state
                                .modify(&RawEntryData::from_entry_data(&record_data))?
                                .state;
                            if key.as_ref() != entry.key.as_ref() && !key.is_placeholder() {
                                create_alias_if_valid(key.as_ref(), &new_row)?;
                            }
                            new_row.commit()?;
                        } else {
                            // we return an error here, since this was an interactive edit
                            state.commit()?;
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
            limit,
            one,
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
                    let attachment_root = get_attachment_root(&data_dir, cli.attachments_dir)?;
                    let mut picker = choose_attachment_path(
                        record_db,
                        template,
                        strict,
                        attachment_root,
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
                FindMode::Records => {
                    let (mut picker, _) = choose_canonical_id(record_db, template, strict, limit);
                    if one {
                        let selection = picker.pick()?;
                        output_find_selection(&selection)?;
                    } else {
                        let selection = picker.pick_multi()?;
                        output_find_selection(&selection)?;
                    }
                }
            }
        }
        Command::Clean { gc_command } => match gc_command {
            cli::GcCommand::Attachments {
                delete_empty: empty,
                migrate,
            } => {
                if empty || migrate {
                    let root_path = get_attachment_root_path(&data_dir, cli.attachments_dir);

                    let mut at_root = AttachmentRoot::open_or_create_unchecked(root_path)?;

                    if migrate {
                        migrate_attachments(&mut at_root)?;
                    }

                    // always cleanup after migration since the migration process may create empty
                    // directories in some cases
                    cleanup_empty_attachment_dirs(&mut at_root)?;
                }
            }
            cli::GcCommand::Database {
                compact,
                evict,
                evict_all,
            } => {
                if let Some(seconds) = evict {
                    record_db.evict_cache_max_age(seconds)?;
                } else if evict_all {
                    record_db.evict_cache()?;
                }

                if compact {
                    record_db.vacuum()?;
                }
            }
        },
        Command::Get {
            identifiers,
            retrieve_only,
            ignore_null,
            template,
            strict,
            sep,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;
            let mut lock = stdout_lock_wrap();

            if cli.read_only {
                if let Some(template) = template {
                    get::retrieve_all_read_only(
                        get::TemplateOutput::new(strict, template, &mut lock, &sep),
                        &cfg,
                        &mut record_db,
                        identifiers,
                        ignore_null,
                    )?;
                } else if retrieve_only {
                    get::retrieve_all_read_only(
                        get::NoOutput,
                        &cfg,
                        &mut record_db,
                        identifiers,
                        ignore_null,
                    )?;
                } else {
                    get::retrieve_all_read_only(
                        get::BibtexOutput::new(&mut lock),
                        &cfg,
                        &mut record_db,
                        identifiers,
                        ignore_null,
                    )?;
                }
            } else {
                if let Some(template) = template {
                    get::retrieve_all(
                        get::TemplateOutput::new(strict, template, &mut lock, &sep),
                        &cfg,
                        client,
                        &mut record_db,
                        identifiers,
                        ignore_null,
                    )?;
                } else if retrieve_only {
                    get::retrieve_all(
                        get::NoOutput,
                        &cfg,
                        client,
                        &mut record_db,
                        identifiers,
                        ignore_null,
                    )?;
                } else {
                    get::retrieve_all(
                        get::BibtexOutput::new(&mut lock),
                        &cfg,
                        client,
                        &mut record_db,
                        identifiers,
                        ignore_null,
                    )?;
                }
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
                    Some((_, DisambiguatedRecordState::Entry(_, state))) => {
                        if revive {
                            error!(
                                "Attempted to redo from a deleted state, but the record currently exists"
                            );
                        } else {
                            hist::handle_redo_result(state.redo(index)?)?;
                        }
                    }
                    Some((_, DisambiguatedRecordState::Deleted(_, state))) => {
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
                    Some((_, DisambiguatedRecordState::Void(_, state))) => {
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
            HistCommand::Reset { identifier, rev } => {
                let cfg = config::load(&config_path, missing_ok)?;
                if let Some((_, disambiguated)) = record_db
                    .state_from_record_id(identifier, &cfg.alias_transform)?
                    .require_record()?
                {
                    let (_, state) = disambiguated.forget();

                    match state.set_active(rev)? {
                        RecordRowMoveResult::Updated(state) => {
                            state.log_opt()?;
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
                    Some((_, DisambiguatedRecordState::Entry(_, state))) => {
                        state.commit()?;
                        bail!("Record already exists!")
                    }
                    Some((_, DisambiguatedRecordState::Deleted(data, state))) => {
                        insert(
                            state,
                            from_bibtex,
                            &data.canonical,
                            cli.no_interactive,
                            &cfg.on_insert,
                            &edit_cmd,
                            None,
                        )?;
                    }
                    Some((_, DisambiguatedRecordState::Void(data, state))) => {
                        insert(
                            state,
                            from_bibtex,
                            &data.canonical,
                            cli.no_interactive,
                            &cfg.on_insert,
                            &edit_cmd,
                            None,
                        )?;
                    }
                    None => {}
                }
            }
            HistCommand::Rewind {
                before,
                target: IdTarget { id, all },
            } => {
                if all {
                    let snapshot = record_db.snapshot()?;
                    snapshot.rewind_all(before)?;
                    snapshot.commit()?;
                } else if let Some(record_id) = id {
                    let cfg = config::load(&config_path, missing_ok)?;
                    if let Some((_, disambiguated)) = record_db
                        .state_from_record_id(record_id, &cfg.alias_transform)?
                        .require_record()?
                    {
                        let (_, state) = disambiguated.forget();
                        let state = state.rewind(before)?;
                        state.log_opt()?;
                        state.commit()?;
                    }
                } else {
                    unreachable!("ArgGroup requires one of these arguments");
                }
            }
            HistCommand::Show { limit } => {
                let snapshot = record_db.snapshot()?;
                let mut stdout = stdout_lock_wrap();
                let styled = stdout.supports_styled_output();
                snapshot.map_history(limit, |record_row, rev_id| {
                    let disp = RecordRowDisplay::from_borrowed_row(record_row, rev_id, styled);
                    writeln!(&mut stdout, "{disp}\n")
                })?;
                snapshot.commit()?;
            }
            HistCommand::Touch {
                target: IdTarget { id, all },
            } => {
                let modified = if all {
                    let snapshot = record_db.snapshot()?;
                    let modified = snapshot.touch_all()?;
                    snapshot.commit()?;
                    modified
                } else if let Some(record_id) = id {
                    let modified = chrono::Local::now();
                    let cfg = config::load(&config_path, missing_ok)?;
                    let (_, row) = get_record_row(&mut record_db, record_id, client, &cfg)?
                        .exists_or_commit_null("Cannot edit")?;
                    row.touch_with_timestamp(&modified)?.commit()?;
                    modified
                } else {
                    unreachable!("ArgGroup requires one of these arguments");
                };
                owriteln!("{modified}")?;
            }
            HistCommand::Undo { identifier, delete } => {
                let cfg = config::load(&config_path, missing_ok)?;
                match record_db
                    .state_from_record_id(identifier, &cfg.alias_transform)?
                    .require_record()?
                {
                    Some((_, DisambiguatedRecordState::Entry(_, state))) => {
                        if delete {
                            hist::handle_undo_result(state.undo_delete()?)?;
                        } else {
                            hist::handle_undo_result(state.undo()?)?;
                        };
                    }
                    Some((_, DisambiguatedRecordState::Deleted(_, state))) => {
                        if delete {
                            hist::handle_undo_result(state.undo_delete()?)?;
                        } else {
                            hist::handle_undo_result(state.undo()?)?;
                        };
                    }
                    Some((_, DisambiguatedRecordState::Void(_, _))) => {
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
                    Some((_, DisambiguatedRecordState::Entry(_, state))) => {
                        state.void()?.commit()?;
                    }
                    Some((_, DisambiguatedRecordState::Deleted(_, state))) => {
                        state.void()?.commit()?;
                    }
                    Some((_, DisambiguatedRecordState::Void(_, state))) => {
                        state.commit()?;
                        error!("Record is already void");
                    }
                    None => {}
                }
            }
        },
        Command::Import {
            targets,
            resolve,
            local_fallback,
            update,
            no_alias,
            include_files,
            file_sep,
        } => {
            let import_config = ImportConfig {
                update,
                resolve,
                local_fallback,
                no_alias,
                file_import_root: if include_files {
                    Some(get_attachment_root(&data_dir, cli.attachments_dir)?)
                } else {
                    None
                },
                file_sep,
            };

            debug!("Using import configuration: {import_config:?}");
            let cfg = config::load(&config_path, missing_ok)?;

            let mut scratch = Vec::new();

            let mut stdout = stdout_lock_wrap();
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
                            &mut stdout,
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
                RecordIdState::Entry(Record { key, row }, state) => {
                    info::database_report(&cfg, key, row, state, report, |_, stdout| {
                        writeln!(stdout, "Record with data")
                    })?;
                }
                RecordIdState::Deleted(Record { key, row }, state) => {
                    info::database_report(&cfg, key, row, state, report, |data, stdout| {
                        if let Some(repl) = data {
                            writeln!(stdout, "Deleted and replaced by reference: {repl}")
                        } else {
                            writeln!(stdout, "Deleted record")
                        }
                    })?;
                }
                RecordIdState::Void(Record { key, row }, state) => {
                    info::database_report(&cfg, key, row, state, report, |_, stdout| {
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
                    InfoReportType::Preferred => {
                        bail!("No preferred keys for null record '{remote_id}'");
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
        Command::List {
            matching,
            canonical,
            deleted,
            template,
            strict,
            sep,
        } => {
            let mut lock = stdout_lock_wrap();
            if let Some(template) = template {
                use get::Output as _;
                if canonical {
                    let mut writer = get::TemplateRowOutput::new(strict, template, &mut lock, &sep);
                    record_db.map_matching_canonical_active_records(&matching, |row_data| {
                        writer.write_item(row_data)
                    })?;
                    writer.finish()?;
                } else {
                    let mut writer = get::TemplateOutput::new(strict, template, &mut lock, &sep);
                    record_db.map_matching_active_records(&matching, |row_data| {
                        writer.write_item(row_data)
                    })?;
                    writer.finish()?;
                }
            } else {
                let snapshot = record_db.snapshot()?;
                if canonical {
                    snapshot.map_canonical_identifiers(deleted, &matching, |key_str| {
                        writeln!(lock, "{key_str}")
                    })?;
                } else {
                    snapshot.map_identifiers(deleted, &matching, |key_str| {
                        writeln!(lock, "{key_str}")
                    })?;
                }
                snapshot.commit()?;
            }
        }
        Command::Local {
            id,
            from_bibtex,
            with_entry_type,
            with_field,
            create_alias,
        } => {
            // check if the provided identifier is a valid alias
            let alias = match Alias::from_str(&id) {
                Ok(alias) => alias,
                Err(e) => match e.kind {
                    AliasErrorKind::Empty => {
                        bail!("local sub-id must contain non-whitespace characters")
                    }
                    AliasErrorKind::IsRemoteId => bail!("local sub-id must not contain a colon"),
                    AliasErrorKind::ContainsControl => {
                        bail!("local sub-id must not contain control characters")
                    }
                },
            };
            let remote_id = RemoteId::local(&alias);
            let edit_cmd = EntryEditCommand {
                update_entry_type: with_entry_type,
                set_field: with_field,
                delete_field: Vec::new(),
            };

            let alias_opt = if create_alias { Some(&alias) } else { None };

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
                        alias_opt,
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
                        alias_opt,
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

            let canonical = match record_db
                .state_from_record_id(identifier, &cfg.alias_transform)?
                .require_record()?
            {
                Some((_, DisambiguatedRecordState::Entry(record_row, _))) => record_row.canonical,
                Some((_, DisambiguatedRecordState::Deleted(record_row, _))) => record_row.canonical,
                Some((_, DisambiguatedRecordState::Void(record_row, _))) => record_row.canonical,
                None => return Ok(()),
            };

            let root = get_attachment_root(&data_dir, cli.attachments_dir)?;
            let mut target = root.attachment_dir(&canonical);
            if mkdir {
                create_dir_all(&target)?;
            }

            // This appends a `/` or `\` when printing, as platform appropriate, to be clear to the
            // user that this is a directory
            target.push("");

            owriteln!("{}", target.display())?;
        }
        Command::Replace {
            identifier,
            with,
            auto,
            hard,
            on_conflict,
            update_aliases,
            ignore_attachments,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;

            if let Some(target) = with {
                let tx = record_db.transaction()?;
                let at_root = if ignore_attachments {
                    None
                } else {
                    get_existing_attachment_root(&data_dir, cli.attachments_dir)?
                };
                replace::replace(
                    identifier,
                    tx,
                    &cfg,
                    |tx, _| {
                        get_record_row_tx(tx, target, client, &cfg)?
                            .exists_or_commit_null("Cannot replace with")
                    },
                    hard,
                    update_aliases,
                    on_conflict,
                    at_root,
                )?;
            } else if auto {
                let tx = record_db.transaction()?;
                let at_root = if ignore_attachments {
                    None
                } else {
                    get_existing_attachment_root(&data_dir, cli.attachments_dir)?
                };
                replace::replace(
                    identifier,
                    tx,
                    &cfg,
                    |tx, data| match determine_key_from_data(data, &cfg) {
                        RemoteIdCandidate::OptimalReference(mapped_key, _)
                        | RemoteIdCandidate::OptimalCanonical(mapped_key) => {
                            let msg = format!(
                                "Automatically determined identifier '{}' is",
                                mapped_key.mapped
                            );
                            get_record_row_tx(tx, mapped_key.mapped.forget(), client, &cfg)?
                                .exists_or_commit_null(&msg)
                        }
                        RemoteIdCandidate::None => {
                            bail!("Could not determine replacement identifier from record data")
                        }
                    },
                    hard,
                    update_aliases,
                    on_conflict,
                    at_root,
                )?;
            } else {
                bail!("Missing replacement target: either use `--with <replacement>` or `--auto`");
            }
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
            json,
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
                if retrieve_only {
                    if cli.read_only {
                        sync_entries_read_only(all_citekeys, &mut record_db, ignore_null, &cfg);
                    } else {
                        sync_entries(all_citekeys, &mut record_db, client, ignore_null, &cfg);
                    }
                } else {
                    let valid_entries = if cli.read_only {
                        retrieve_entries_read_only(all_citekeys, &mut record_db, ignore_null, &cfg)
                    } else {
                        retrieve_entries(all_citekeys, &mut record_db, client, ignore_null, &cfg)
                    };

                    if json {
                        output_entries_json(&valid_entries)?;
                    } else {
                        output_entries_bibtex(outfile, append, &valid_entries)?;
                    }
                }
            }
        }
        Command::Update {
            identifier,
            from_bibtex,
            from_record,
            from_rev,
            on_conflict,
            revive,
        } => {
            let cfg = config::load(&config_path, missing_ok)?;
            let tx = record_db.transaction()?;

            // this has to be done first since we need a mutable reference to
            // record_db, which we cannot use once we start the update
            // routine. However, we do not determine the data in the other cases
            // at this point since we would like to defer filesystem / network
            // operations, unless they are strictly required
            let (provided_data, tx) = if let Some(record_id) = from_record {
                let (data, tx) = data_from_key(tx, record_id, &cfg)?;
                (Some(data), tx)
            } else if let Some(rev) = from_rev {
                let data = data_from_rev(&tx, rev)?;
                (Some(data), tx)
            } else {
                (None, tx)
            };

            update(
                on_conflict,
                RecordIdState::determine(tx, identifier, &cfg.alias_transform)?,
                provided_data,
                &cfg.on_insert,
                revive,
                |canonical| {
                    if let Some(path) = from_bibtex {
                        Ok(data_from_path(path)?)
                    } else if canonical.is_local() {
                        bail!(
                            "Cannot update local record using remote data: use `autobib edit` or the `--from-bibtex` or `--from-key` options."
                        );
                    } else {
                        Ok(update::data_from_remote(canonical, client)?.0)
                    }
                },
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
            UtilCommand::List { .. } => {
                bail!("`autobib util list` is deprecated; use `autobib list` instead");
            }
            UtilCommand::Optimize => {
                bail!(
                    "`autobib util optimize` is deprecated; use `autobib clean database --compact`"
                );
            }
            UtilCommand::Evict { .. } => {
                bail!("`autobib util evict` is deprecated; use `autobib clean database`");
            }
        },
    };

    Ok(())
}
