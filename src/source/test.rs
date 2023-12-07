use super::*;
use biblatex::{Bibliography, Entry};

pub struct TestRecordSource;

impl RecordSource for TestRecordSource {
    const SOURCE_NAME: &'static str = "test";

    fn is_valid_id(&self, id: &str) -> bool {
        match id {
            "005" => false,
            _ => true,
        }
    }

    fn get_record(&self, id: &str) -> Result<Option<Entry>, RecordError> {
        // Assume `id` is valid
        match id {
            "003" => {
                let raw =
                    "@article{test:003, author = {Two, Author and One, Author}, title = {Example}}";
                let bibliography = Bibliography::parse(raw).unwrap();
                let entry = bibliography.get("test:003").unwrap();

                Ok(Some(entry.clone()))
            }
            "004" => Ok(None),
            _ => Err(RecordError::Incomplete),
        }
    }
}
