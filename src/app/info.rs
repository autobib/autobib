use std::io::Write;

use itertools::Itertools;
use regex::Regex;
use serde_bibtex::token::is_entry_key;

use crate::{
    app::cli::InfoReportType,
    config::Config,
    db::state::{InRecordsTable, RecordRow, State},
    logger::error,
    output::{StdoutWriter, owriteln, stdout_lock_wrap},
};

/// Get the preferred identifier associated with a record in the Records table.
fn get_preferred_id<'conn, D, I: InRecordsTable>(
    state: &State<'conn, I>,
    data: RecordRow<D>,
    preferred_providers: &[String],
) -> anyhow::Result<crate::RemoteId> {
    if !preferred_providers.is_empty() {
        let mut referencing_ids = state.referencing_remote_ids()?;
        referencing_ids.sort_unstable_by(|l, r| l.provider().cmp(r.provider()));
        for provider in preferred_providers {
            if let Ok(idx) = referencing_ids.binary_search_by(|id| id.provider().cmp(provider)) {
                return Ok(referencing_ids.swap_remove(idx));
            }
        }
    }

    Ok(data.canonical)
}

pub fn database_report<'conn, D, I, F>(
    config: &Config<F>,
    record_id: String,
    data: RecordRow<D>,
    state: State<'conn, I>,
    report: InfoReportType,
    header: impl FnOnce(D, &mut StdoutWriter) -> std::io::Result<()>,
) -> anyhow::Result<()>
where
    I: InRecordsTable,
    F: FnOnce() -> Vec<(Regex, String)>,
{
    match report {
        InfoReportType::All => {
            let mut lock = stdout_lock_wrap();
            header(data.data, &mut lock)?;
            writeln!(lock, "Canonical: {}", data.canonical)?;
            writeln!(lock, "Revision: {}", state.rev())?;
            writeln!(
                lock,
                "Equivalent references: {}",
                state.referencing_keys()?.iter().join(", ")
            )?;
            writeln!(
                lock,
                "Valid BibTeX? {}",
                if is_entry_key(&record_id) {
                    "yes"
                } else {
                    "no"
                }
            )?;
            writeln!(lock, "Data last modified: {}", data.modified)?;
        }
        InfoReportType::Canonical => {
            owriteln!("{}", state.canonical()?)?;
        }

        InfoReportType::Valid => {
            if !is_entry_key(&record_id) {
                error!("Invalid BibTeX: {record_id}");
            }
        }
        InfoReportType::Revision => {
            owriteln!("{}", state.rev())?;
        }
        InfoReportType::Preferred => {
            owriteln!(
                "{}",
                get_preferred_id(&state, data, &config.preferred_providers)?
            )?;
        }
        InfoReportType::Equivalent => {
            let mut lock = stdout_lock_wrap();
            for re in state.referencing_keys()? {
                writeln!(lock, "{re}")?;
            }
        }
        InfoReportType::Modified => {
            owriteln!("{}", state.last_modified()?)?;
        }
    };
    state.commit()?;
    Ok(())
}
