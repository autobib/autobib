use super::*;
use crate::entry::{Entry, Fields};

pub fn is_valid_id(id: &str) -> bool {
    match id {
        "005" => false,
        _ => true,
    }
}

pub fn get_record(id: &str) -> Result<Option<Entry>, RecordError> {
    // Assume `id` is valid
    match id {
        "003" => Ok(Some(Entry {
            entry_type: "article".to_string(),
            fields: Fields {
                author: Some("Two, Author and One, Author".to_string()),
                title: Some("Example".to_string()),
                ..Fields::default()
            },
        })),
        "004" => Ok(None),
        _ => Err(RecordError::Incomplete),
    }
}
