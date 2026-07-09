use std::cmp::Ordering;

use serde::Serialize;
use serde_bibtex::token::is_entry_key;

use crate::{
    config::Config,
    db::state::{ArbitraryData, InRecordsTable, RecordRow, RevisionId, State},
};

#[derive(Serialize)]
pub struct KeyInfo {
    original: String,
    is_valid_bibtex: bool,
    preferred: Option<String>,
    equivalent: Vec<String>,
}

#[derive(Serialize)]
pub struct RecordInfo {
    pub key: KeyInfo,
    pub id: RevisionId,
    pub record: RecordRow<ArbitraryData>,
}

impl RecordInfo {
    pub fn from_data<'conn, I: InRecordsTable>(
        original: String,
        record: RecordRow<ArbitraryData>,
        state: &State<'conn, I>,
        config: &Config,
    ) -> anyhow::Result<Self> {
        let is_valid_bibtex = is_entry_key(&original);
        let preferred = get_preferred_id(state, config)?;
        let equivalent = state.referencing_keys()?;
        let key = KeyInfo {
            original,
            is_valid_bibtex,
            preferred,
            equivalent,
        };
        let id = state.rev();
        Ok(Self { key, id, record })
    }
}

/// Get the preferred identifier associated with a record in the Records table, or `None` if no
/// identifier matches.
pub fn get_preferred_id<'conn, I: InRecordsTable>(
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
