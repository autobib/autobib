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
    let mut record_db = create_test_db().unwrap();

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
            record_db.get(record_id).map_or_else(
                // error retrieving record
                |err| {
                    eprintln!("{}", err);
                    None
                },
                |record| {
                    record.data.or_else(|| {
                        // null record
                        eprintln!("Warning: '{}' is a null record!", record.id);
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

use crate::db::Record;
use crate::record::RecordError;
use biblatex::Bibliography;

/// Populate the database with some records for testing purposes.
fn create_test_db() -> Result<RecordDatabase, RecordError> {
    let mut record_db = RecordDatabase::create(DATABASE_FILE)?;

    let raw = "@article{test:000, author = {Rutar, Alex and Wu, Peiran}, title = {Autobib}}";
    let bibliography = Bibliography::parse(raw).unwrap();
    let entry = bibliography.get("test:000").unwrap();
    record_db.set_cached_data(&Record::new(
        RecordId::from_str("test:000").unwrap(),
        Some(entry.clone()),
    ))?;

    let raw2 = "@article{test:002, author = {Author, Test}, title = {A Sample Paper}}";
    let bibliography2 = Bibliography::parse(raw2).unwrap();
    let entry2 = bibliography2.get("test:002").unwrap();
    record_db.set_cached_data(&Record::new(
        RecordId::from_str("test:002").unwrap(),
        Some(entry2.clone()),
    ))?;

    record_db.set_cached_data(&Record::new(RecordId::from_str("test:001").unwrap(), None))?;

    Ok(record_db)
}
