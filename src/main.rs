mod api;
mod database;
mod entry;
mod record;
mod source;

use std::str::FromStr;

use clap::Parser;
use rusqlite::Result;

use api::*;
use entry::KeyedEntry;

const DATABASE_FILE: &str = "cache.db";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    args: Vec<String>,
}

fn main() {
    let cli = Cli::parse();

    // Initialize database
    let mut record_db = create_test_db().unwrap();

    // Collect all entries which are not null
    let valid_entries: Vec<KeyedEntry> = cli
        .args
        .into_iter()
        // parse the source:sub_id arguments
        .filter_map(|input| match CitationKey::from_str(&input) {
            Ok(record_id) => Some(record_id),
            Err(err) => {
                eprintln!("{err}");
                None
            }
        })
        // perform "cheap" record_id validation
        .filter(|record_id| match validate_citation_key(record_id) {
            ValidationResult::InvalidSource(s) => {
                eprintln!("invalid source: '{s}'");
                false
            }
            ValidationResult::InvalidSubId(s) => {
                eprintln!("invalid sub-id: '{s}'");
                false
            }
            ValidationResult::Ok => true,
        })
        // retrieve records
        .filter_map(|citation_key| {
            get_record(&mut record_db, &citation_key).map_or_else(
                // error retrieving record
                |err| {
                    eprintln!("{err}");
                    None
                },
                |response| match response {
                    Some(entry) => Some(KeyedEntry {
                        key: citation_key,
                        contents: entry,
                    }),
                    None => {
                        eprintln!("'null record: {citation_key}'");
                        None
                    }
                },
            )
        })
        .collect();

    // print biblatex strings
    for entry in valid_entries {
        println!("{}", entry)
    }
}

/// Populate the database with some records for testing purposes.
fn create_test_db() -> Result<RecordDatabase, RecordError> {
    use entry::{Entry, Fields};
    match std::fs::remove_file(DATABASE_FILE) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => panic!("Testing database file has been overwritten!"),
    }

    let mut record_db = RecordDatabase::create(DATABASE_FILE)?;

    let entry_1 = Entry {
        entry_type: "code".to_string(),
        fields: Fields {
            author: Some("Rutar, Alex and Wu, Peiran".to_string()),
            title: Some("Autobib".to_string()),
            ..Fields::default()
        },
    };
    record_db.set_cached_data(&RecordId::from_str("test:000").unwrap(), &entry_1, None)?;

    let entry_2 = Entry {
        entry_type: "article".to_string(),
        fields: Fields {
            author: Some("Author, Test".to_string()),
            title: Some("A Sample Paper".to_string()),
            ..Fields::default()
        },
    };
    record_db.set_cached_data(&RecordId::from_str("test:002").unwrap(), &entry_2, None)?;

    Ok(record_db)
}
