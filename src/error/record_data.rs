use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum RecordDataError {
    #[error("Key is not ASCII lowercase `[a-z]`.")]
    KeyNotAsciiLowercase,
    #[error("Key has invalid size {0}; must be at least 1 and at most `u8::MAX`.")]
    KeyInvalidLength(usize),
    #[error("Entry type is not ASCII lowercase `[a-z]`.")]
    EntryTypeNotAsciiLowercase,
    #[error("Entry type has invalid size {0}; must be at least 1 and at most `u8::MAX`.")]
    EntryTypeInvalidLength(usize),
    #[error("Value has invalid size {0}; must be at most `u16::MAX`.")]
    ValueInvalidLength(usize),
    #[error("Value does not contain balanced `{{ }}` brackets.")]
    ValueNotBalanced,
    #[error("Record data structure contains maximum number of entries.")]
    RecordDataFull,
}
