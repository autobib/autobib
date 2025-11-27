use std::io::Write;

use itertools::Itertools;
use serde_bibtex::token::is_entry_key;

use crate::{
    app::cli::InfoReportType,
    db::state::{InRecordsTable, RecordRow, State},
    logger::error,
    output::{StdoutWriter, owriteln, stdout_lock_wrap},
};

pub fn database_report<'conn, D, I: InRecordsTable>(
    record_id: String,
    data: RecordRow<D>,
    state: State<'conn, I>,
    report: InfoReportType,
    header: impl FnOnce(D, &mut StdoutWriter) -> std::io::Result<()>,
) -> anyhow::Result<()> {
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
