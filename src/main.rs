use crate::db::RecordDatabase;
use crate::record::RecordId;
use biblatex::Entry;
use clap::Parser;
use rusqlite::Result;
use std::str::FromStr;

mod db;
mod record;
mod share {
    pub mod test;
}

const DATABASE_FILE: &str = "cache.db";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    args: Vec<String>,
}

fn main() -> Result<(), RecordError> {
    let cli = Cli::parse();

    // Initialize database
    let record_db = RecordDatabase::try_new(DATABASE_FILE)?;
    test_populate_db(&record_db)?; // TODO: remove

    // Collect all entries which are not null
    let valid_entries: Vec<Entry> = cli
        .args
        .into_iter()
        // parse the source:sub_id arguments
        .filter_map(|input| match RecordId::from_str(&input) {
            Ok(record_id) => Some(record_id),
            Err(error) => {
                eprintln!("{}", error);
                None
            }
        })
        .filter_map(|record_id| {
            record_db.get(&record_id).map_or_else(
                // error retrieving record
                |err| {
                    eprintln!("{}", err);
                    None
                },
                |record_cache| {
                    record_cache.record.or_else(|| {
                        // null record
                        eprintln!("Warning: '{}' is a null record!", record_id);
                        None
                    })
                },
            )
        })
        .collect();

    // print biblatex strings
    for entry in valid_entries {
        println!("{}", entry.to_biblatex_string())
    }

    Ok(())
}

use crate::record::{Record, RecordError};
use biblatex::Bibliography;

/// Populate the database with some records for testing purposes.
fn test_populate_db(record_db: &RecordDatabase) -> Result<(), RecordError> {
    let raw = "@article{test:000, author = {Rutar, Alex and Wu, Peiran}, title = {Autobib}}";
    let bibliography = Bibliography::parse(raw).unwrap();
    let entry = bibliography.get("test:000").unwrap();
    record_db.set_cached(
        &RecordId::from_str("test:000").unwrap(),
        &Record::new(Some(entry.clone())),
    )?;

    let raw2 = "@article{test:002, author = {Author, Test}, title = {A Sample Paper}}";
    let bibliography2 = Bibliography::parse(raw2).unwrap();
    let entry2 = bibliography2.get("test:002").unwrap();
    record_db.set_cached(
        &RecordId::from_str("test:002").unwrap(),
        &Record::new(Some(entry2.clone())),
    )?;

    record_db.set_cached(&RecordId::from_str("test:001").unwrap(), &Record::new(None))?;

    Ok(())
}
