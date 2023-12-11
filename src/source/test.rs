use super::*;
use crate::entry::{AnonymousEntry, Fields};

pub struct TestRecordSource;

impl RecordSource for TestRecordSource {
    fn is_valid_id(&self, id: &str) -> bool {
        match id {
            "005" => false,
            _ => true,
        }
    }

    fn get_record(&self, id: &str) -> Result<Option<AnonymousEntry>, RecordError> {
        // Assume `id` is valid
        match id {
            "003" => Ok(Some(AnonymousEntry {
                entry_type: "article".to_string(),
                fields: Fields {
                    author: Some("Two, Author and One, Author".to_string()),
                    title: Some("Example".to_string()),
                },
            })),
            "004" => Ok(None),
            _ => Err(RecordError::Incomplete),
        }
    }
}
