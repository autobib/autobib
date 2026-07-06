use std::{cmp::Ordering, io::Write};

use itertools::Itertools;
use serde_bibtex::token::is_entry_key;

use crate::{
    app::cli::InfoReportType,
    config::Config,
    db::state::{InRecordsTable, RecordRow, State},
    logger::error,
    output::{StdoutWriter, owriteln, stdout_lock_wrap},
};

/// Get the preferred identifier associated with a record in the Records table, or `None` if no
/// identifier matches.
fn get_preferred_id<'conn, I: InRecordsTable>(
    state: &State<'conn, I>,
    config: &Config,
) -> anyhow::Result<Option<String>> {
    if config.has_preferred_keys() {
        let mut best: Option<(String, usize)> = None;
        state.map_referencing_keys(|new| {
            if let Some(new_score) = config.preferred_key_matching_idx(new) {
                if let Some((best_s, best_score)) = best.as_mut() {
                    match new_score.cmp(best_score) {
                        Ordering::Less => {
                            // new score is better
                            best_s.clear();
                            best_s.push_str(new);
                            *best_score = new_score;
                        }
                        Ordering::Equal => {
                            // break ties lexicographically
                            if *new < **best_s {
                                best_s.clear();
                                best_s.push_str(new);
                            }
                        }
                        Ordering::Greater => {}
                    }
                } else {
                    best = Some((new.to_owned(), new_score));
                }
            }
        })?;
        return Ok(best.map(|(s, _)| s));
    }

    Ok(None)
}

pub fn database_report<'conn, D, I>(
    config: &Config,
    record_id: String,
    data: RecordRow<D>,
    state: State<'conn, I>,
    report: InfoReportType,
    header: impl FnOnce(D, &mut StdoutWriter) -> std::io::Result<()>,
) -> anyhow::Result<()>
where
    I: InRecordsTable,
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
            if let Some(s) = get_preferred_id(&state, config)? {
                owriteln!("{s}")?;
            } else {
                owriteln!("{}", data.canonical)?;
            }
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
