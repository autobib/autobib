use crate::share::test::TestRecordSource;
use biblatex::Entry;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::fmt;
use std::str::FromStr;
use std::string::ToString;

/// Various failure modes for records.
#[derive(Debug)]
pub enum RecordError {
    InvalidRepoIdFormat(String),
    InvalidRepository(RepoId),
    InvalidId(RepoId),
    DatabaseFailure(rusqlite::Error),
    Incomplete,
}

impl From<rusqlite::Error> for RecordError {
    fn from(err: rusqlite::Error) -> Self {
        RecordError::DatabaseFailure(err)
    }
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordError::InvalidRepoIdFormat(input) => {
                write!(
                    f,
                    "'{}' is not in the format of '<repository>:<id>'.",
                    input
                )
            }
            RecordError::InvalidRepository(repo_id) => {
                write!(f, "'{}' is not a valid repository.", repo_id.repo)
            }
            RecordError::InvalidId(repo_id) => write!(
                f,
                "'{}' is not a valid id in the repository '{}'.",
                repo_id.id, repo_id.repo
            ),
            RecordError::Incomplete => write!(f, "Incomplete record"),
            RecordError::DatabaseFailure(error) => write!(f, "Database failure: {}", error),
        }
    }
}

/// A source (`repo`) with corresponding identity (`id`), such as arxiv:0123.4567
#[derive(Debug, Clone, Hash, PartialEq, Eq, DeserializeFromStr, SerializeDisplay)]
pub struct RepoId {
    pub repo: String,
    pub id: String,
}

impl FromStr for RepoId {
    type Err = RecordError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let first_colon_position = input.find(':');
        match first_colon_position {
            Some(p) => {
                if p == 0 || p == input.len() - 1 {
                    return Err(RecordError::InvalidRepoIdFormat(input.to_string()));
                }
                let repo = String::from(&input[0..p]);
                let id = String::from(&input[p + 1..]);
                Ok(RepoId { repo, id })
            }
            None => Err(RecordError::InvalidRepoIdFormat(input.to_string())),
        }
    }
}

impl fmt::Display for RepoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.repo, self.id)
    }
}

/// An individual record, which also caches non-existence of the entry.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Record {
    pub repo_id: RepoId,
    pub record: Option<Entry>,
    pub accessed: DateTime<Local>,
}

impl Record {
    pub fn new(repo_id: RepoId, record: Option<Entry>) -> Self {
        Self {
            repo_id,
            record,
            accessed: Local::now(),
        }
    }

    pub fn to_param<'a>(&'a self) -> (String, String, &'a DateTime<Local>) {
        // TODO: proper error here
        (
            self.repo_id.to_string(),
            serde_json::to_string(&self.record).unwrap(),
            &self.accessed,
        )
    }

    pub fn from_row(repo_id: &RepoId, row: &rusqlite::Row) -> Result<Self, rusqlite::Error> {
        let record_cache_str: String = row.get("record")?;
        let accessed: DateTime<Local> = row.get("accessed")?;
        Ok(Record {
            repo_id: repo_id.clone(),
            record: serde_json::from_str(&record_cache_str).unwrap(),
            accessed,
        })
    }
}

/// Determine the record source corresponding to the repo.
pub fn lookup_record_source(repo_id: &RepoId) -> Result<impl RecordSource, RecordError> {
    match repo_id.repo.as_str() {
        "test" => Ok(TestRecordSource {}),
        _ => Err(RecordError::InvalidRepository(repo_id.clone())),
    }
}

/// A RecordSource is, abstractly, a lookup function
pub trait RecordSource {
    const REPO: &'static str;

    fn is_valid_id(&self, id: &str) -> bool;
    fn get_record(&self, id: &str) -> Result<Option<Entry>, RecordError>;
}
