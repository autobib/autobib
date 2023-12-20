use super::*;
use crate::entry::{Entry, Fields};

pub struct TestRecordSource;

impl RecordSource for TestRecordSource {
    fn is_valid_id(&self, id: &str) -> bool {
        match id {
            "005" => false,
            _ => true,
        }
    }

    fn get_record(&self, id: &str) -> Result<Option<Entry>, RecordError> {
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
}
