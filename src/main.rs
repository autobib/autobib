use serde::{Deserialize, Serialize};
use std::error::Error;

mod arxiv;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let repository = &args[1];
    let id = &args[2];
    let result = get_record(repository, id);
    // Create an empty list of records
    let mut records: Vec<Record> = Vec::new();
    match result {
        Ok(record) => {
            println!("{:?}", record);
            // Add the record to the list
            records.push(record);
            let yaml = serde_yaml::to_string(&records).unwrap();
            std::fs::write(CACHE_FILE, yaml).unwrap();
        }
        Err(error) => match error {
            RecordError::InvalidRepository => println!("Invalid repository"),
            RecordError::InvalidId => println!("Invalid id"),
            RecordError::Other(message) => println!("Other error: {}", message),
        },
    }
    // let yaml = std::fs::read_to_string(CACHE_FILE).unwrap();
    // let records: Vec<Record> = serde_yaml::from_str(&yaml).unwrap();
    // println!("{:?}", records);
}

const CACHE_FILE: &str = "cache.yaml";

fn get_record(repository: &str, id: &str) -> Result<Record, RecordError> {
    if repository == "arxiv" {
        let validation_result = arxiv::validate_id(id);
        if validation_result == ValidationResult::Invalid {
            return Err(RecordError::InvalidId);
        }
        return arxiv::get_record(id);
    } else {
        return Err(RecordError::InvalidRepository);
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    repository: String,
    id: String,
    title: String,
    authors: Vec<String>,
}

#[derive(PartialEq, Eq, Debug, Serialize, Deserialize)]
enum ValidationResult {
    Valid,
    Invalid,
}

#[derive(Debug, Serialize, Deserialize)]
enum RecordError {
    InvalidRepository,
    InvalidId,
    Other(String),
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl Error for RecordError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}
