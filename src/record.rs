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

/// A valid repo:id has precisely one colon `:`
impl FromStr for RepoId {
    type Err = RecordError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = input.split(':').collect();

        if parts.len() == 2 {
            let repo = parts[0].to_string();
            let id = parts[1].to_string();
            Ok(RepoId { repo, id })
        } else {
            Err(RecordError::InvalidRepoIdFormat(input.to_string()))
        }
    }
}

/// Display as repo:id
impl fmt::Display for RepoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.repo, self.id)
    }
}

/// An individual record, which also caches non-existence of the entry.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Record {
    pub record: Option<Entry>,
    pub accessed: DateTime<Local>,
}

impl Record {
    pub fn new(record: Option<Entry>) -> Self {
        Self {
            record,
            accessed: Local::now(),
        }
    }

    pub fn to_param<'a>(&'a self, repo_id: &RepoId) -> (String, String, &'a DateTime<Local>) {
        // TODO: proper error here
        (
            repo_id.to_string(),
            serde_json::to_string(&self.record).unwrap(),
            &self.accessed,
        )
    }

    pub fn from_row(row: &rusqlite::Row) -> Result<Self, rusqlite::Error> {
        let record_cache_str: String = row.get(0)?;
        let accessed: DateTime<Local> = row.get(1)?;
        Ok(Record {
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
