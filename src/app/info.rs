use std::{cmp::Ordering, fmt};

use autobib_entry::data::EntryData;
use serde::Serialize;
use serde_bibtex::token::is_entry_key;

use crate::{
    config::Config,
    db::state::{ArbitraryData, InRecordsTable, Record, RevisionId, State},
};

#[derive(Serialize)]
pub struct KeyInfo {
    original: String,
    is_valid_bibtex: bool,
    user_preferred: Option<String>,
    equivalent: Vec<String>,
}

#[derive(Serialize)]
pub struct RecordInfo {
    pub key: KeyInfo,
    pub revision: RevisionId,
    pub record: Record<ArbitraryData>,
}

impl fmt::Display for RecordInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Key: {}", self.key.original)?;
        writeln!(f, "Revision: {}", self.revision)?;
        writeln!(f, "Last modified: {}", self.record.modified)?;
        writeln!(f, "==> Key and identifier")?;
        writeln!(f, "Canonical identifier: {}", self.record.canonical)?;
        if let Some(ref preferred) = self.key.user_preferred {
            writeln!(f, "Preferred key: {preferred}")?;
        } else {
            writeln!(f, "No matching preferred key")?;
        }
        writeln!(
            f,
            "Valid bibtex? {}",
            if self.key.is_valid_bibtex {
                "yes"
            } else {
                "no"
            }
        )?;
        writeln!(f, "Equivalent keys:")?;
        for k in &self.key.equivalent {
            writeln!(f, "  {k}")?;
        }
        match &self.record.data {
            ArbitraryData::Entry(raw_entry_data) => {
                writeln!(f, "==> Entry data",)?;
                writeln!(f, "Entry type: {}", raw_entry_data.entry_type())?;
                writeln!(f, "Fields:",)?;
                for (k, v) in raw_entry_data.fields() {
                    writeln!(f, "  {k} = {{{v}}}")?;
                }
            }
            ArbitraryData::Deleted(id) => {
                writeln!(f, "==> Soft-deleted",)?;
                if let Some(s) = id {
                    writeln!(f, "Replaced by: {s}")?;
                } else {
                    writeln!(f, "No replacement key")?;
                }
            }
            ArbitraryData::Void => {
                writeln!(f, "==> Void")?;
            }
        }
        Ok(())
    }
}

impl RecordInfo {
    pub fn from_data<'conn, I: InRecordsTable>(
        original: String,
        record: Record<ArbitraryData>,
        state: &State<'conn, I>,
        config: &Config,
    ) -> anyhow::Result<Self> {
        let is_valid_bibtex = is_entry_key(&original);
        let user_preferred = get_user_preferred_id(state, config)?;
        let equivalent = state.referencing_keys()?;
        let key = KeyInfo {
            original,
            is_valid_bibtex,
            user_preferred,
            equivalent,
        };
        let revision = state.rev();
        Ok(Self {
            key,
            revision,
            record,
        })
    }
}

/// Get the preferred identifier associated with a record in the Records table, or `None` if no
/// identifier matches.
pub fn get_user_preferred_id<'conn, I: InRecordsTable>(
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
