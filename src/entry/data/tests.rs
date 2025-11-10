use super::*;

use crate::error::{InvalidBytesError, RecordDataError};

#[test]
fn test_normalize_whitespace() {
    let mut record_data = MutableEntryData::try_new("article".into()).unwrap();
    for (k, v) in [("a", " "), ("b", "ok"), ("c", "a\t b")] {
        record_data.check_and_insert(k.into(), v.into()).unwrap();
    }
    let changed = record_data.normalize_whitespace();
    assert!(changed);
    assert_eq!(record_data.get_str("a"), Some(""));
    assert_eq!(record_data.get_str("b"), Some("ok"));
    assert_eq!(record_data.get_str("c"), Some("a b"));
    assert!(!record_data.normalize_whitespace());
}

#[test]
fn test_normalize_eprint() {
    // standard normalize
    let mut record_data = MutableEntryData::try_new("article".into()).unwrap();
    for (k, v) in [
        ("doi", "xxx"),
        ("eprinttype", "doi"),
        ("eprint", "xxx"),
        ("zbl", "yyy"),
    ] {
        record_data.check_and_insert(k.into(), v.into()).unwrap();
    }
    let changed = record_data.set_eprint(["zbl", "doi"].iter());
    assert!(changed);
    assert_eq!(record_data.get_str("eprint"), Some("yyy"));
    assert_eq!(record_data.get_str("eprinttype"), Some("zbl"));

    // already ok
    let mut record_data = MutableEntryData::try_new("article".into()).unwrap();
    for (k, v) in [
        ("doi", "xxx"),
        ("eprinttype", "doi"),
        ("eprint", "xxx"),
        ("zbl", "yyy"),
    ] {
        record_data.check_and_insert(k.into(), v.into()).unwrap();
    }
    let changed = record_data.set_eprint(["doi", "zbl"].iter());
    assert!(!changed);

    // set new
    let mut record_data = MutableEntryData::try_new("article".into()).unwrap();
    for (k, v) in [("doi", "xxx"), ("zbl", "yyy")] {
        record_data.check_and_insert(k.into(), v.into()).unwrap();
    }
    let changed = record_data.set_eprint(["zbl", "doi"].iter());
    assert!(changed);
    assert_eq!(record_data.get_str("eprint"), Some("yyy"));
    assert_eq!(record_data.get_str("eprinttype"), Some("zbl"));

    // set new partial
    let mut record_data = MutableEntryData::try_new("article".into()).unwrap();
    for (k, v) in [("doi", "xxx"), ("eprint", "xxx")] {
        record_data.check_and_insert(k.into(), v.into()).unwrap();
    }
    let changed = record_data.set_eprint(["zbl", "doi"].iter());
    assert!(changed);
    assert_eq!(record_data.get_str("eprint"), Some("xxx"));
    assert_eq!(record_data.get_str("eprinttype"), Some("doi"));

    // skip missing without changing
    let mut record_data = MutableEntryData::try_new("article".into()).unwrap();
    for (k, v) in [("doi", "xxx"), ("eprint", "xxx"), ("eprinttype", "doi")] {
        record_data.check_and_insert(k.into(), v.into()).unwrap();
    }
    let changed = record_data.set_eprint(["zbl", "doi"].iter());
    assert!(!changed);

    // set new skip
    let mut record_data = MutableEntryData::try_new("article".into()).unwrap();
    {
        let (k, v) = ("doi", "xxx");
        record_data.check_and_insert(k.into(), v.into()).unwrap();
    }
    let changed = record_data.set_eprint(["zbl", "doi"].iter());
    assert!(changed);
    assert_eq!(record_data.get_str("eprint"), Some("xxx"));
    assert_eq!(record_data.get_str("eprinttype"), Some("doi"));

    // skip
    let mut record_data = MutableEntryData::try_new("article".into()).unwrap();
    for (k, v) in [("zbl", "yyy"), ("eprinttype", "doi")] {
        record_data.check_and_insert(k.into(), v.into()).unwrap();
    }
    let changed = record_data.set_eprint(["doi"].iter());
    assert!(!changed);

    // no data skip
    let mut record_data = MutableEntryData::try_new("article".into()).unwrap();
    let changed = record_data.set_eprint(["doi"].iter());
    assert!(!changed);

    // no match multi skip
    let mut record_data = MutableEntryData::try_new("article".into()).unwrap();
    for (k, v) in [("zbl", "yyy"), ("eprinttype", "doi")] {
        record_data.check_and_insert(k.into(), v.into()).unwrap();
    }
    let changed = record_data.set_eprint(["doi", "zbmath"].iter());
    assert!(!changed);
}

/// Check that conversion into the raw form and back results in identical data.
#[test]
fn test_data_round_trip() {
    let mut record_data = MutableEntryData::try_new("article".into()).unwrap();
    record_data
        .check_and_insert("year".into(), "2024".into())
        .unwrap();
    record_data
        .check_and_insert("title".into(), "A title".into())
        .unwrap();
    record_data
        .check_and_insert("field".into(), "".into())
        .unwrap();
    record_data
        .check_and_insert("a".repeat(255), "🍄".into())
        .unwrap();
    record_data
        .check_and_insert("a".into(), "b".repeat(65_535))
        .unwrap();

    let raw_data = RawEntryData::from_entry_data(&record_data);

    let mut record_data_clone = MutableEntryData::try_new(raw_data.entry_type().into()).unwrap();

    for (key, value) in raw_data.fields() {
        record_data_clone
            .check_and_insert(key.into(), value.into())
            .unwrap();
    }

    assert_eq!(record_data, record_data_clone);
    assert_eq!(
        raw_data.to_byte_repr(),
        RawEntryData::from_entry_data(&record_data_clone).to_byte_repr()
    );
}

#[test]
fn test_insert_len() {
    let mut record_data = MutableEntryData::try_new("a".into()).unwrap();

    assert_eq!(
        record_data.check_and_insert("a".repeat(256), "".into()),
        Err(RecordDataError::KeyInvalidLength(256))
    );

    assert_eq!(
        record_data.check_and_insert("a".into(), "🍄".repeat(20_000)),
        Err(RecordDataError::ValueInvalidLength(80_000))
    );

    assert_eq!(
        record_data.check_and_insert("".into(), "".into()),
        Err(RecordDataError::KeyInvalidLength(0))
    );

    assert!(
        record_data
            .check_and_insert("a".repeat(255), "".into())
            .is_ok(),
    );
}

#[test]
fn test_round_trip() {
    fn check(keys: &[(&'static str, &'static str)]) {
        let mut data = MutableEntryData::<String>::default();
        for (k, v) in keys {
            data.check_and_insert((*k).into(), (*v).into()).unwrap();
        }
        assert_eq!(data.fields().count(), keys.len());

        let raw_data = RawEntryData::from_entry_data(&data);
        assert_eq!(raw_data.fields().count(), keys.len());

        let new_data = MutableEntryData::from_entry_data(&raw_data);
        assert_eq!(new_data.fields().count(), keys.len());

        for (k, v) in keys {
            assert_eq!(raw_data.get_field(k), Some(*v));
            assert_eq!(data.get_field(k), Some(*v));
            assert_eq!(new_data.get_field(k), Some(*v));
        }
    }
    check(&[("a", "A"), ("b", "B")]);
    check(&[("a", "A"), ("c", ""), ("b", "C")]);
    check(&[]);
    check(&[("b", "a")]);
}

#[test]
fn test_format_manual() {
    let mut record_data = MutableEntryData::try_new("article".into()).unwrap();
    record_data
        .check_and_insert("year".into(), "2023".into())
        .unwrap();
    record_data
        .check_and_insert("title".into(), "The Title".into())
        .unwrap();

    let data = RawEntryData::from_entry_data(&record_data);
    let expected = vec![
        0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 5, 9, 0, b't', b'i', b't', b'l', b'e',
        b'T', b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e', b'a', b'r',
        b'2', b'0', b'2', b'3',
    ];

    assert_eq!(expected, data.to_byte_repr());
}

#[test]
fn test_validate_data_ok() {
    for data in [
        // usual example
        vec![
            0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 5, 9, 0, b't', b'i', b't', b'l', b'e',
            b'T', b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e', b'a', b'r',
            b'2', b'0', b'2', b'3',
        ],
        // no keys is OK
        vec![0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e'],
        // field value can have length 0
        vec![0, 1, b'a', 1, 0, 0, b'b'],
        // usual example
        vec![
            0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 5, 9, 0, b't', b'i', b't', b'l', b'e',
            b'T', b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e', b'a', b'r',
            b'2', b'0', b'2', b'3',
        ],
    ] {
        assert!(RawEntryData::from_byte_repr(data).is_ok());
    }
}

#[test]
fn test_validate_data_err() {
    // invalid version
    let malformed_data = vec![
        2, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 5, 9, 0, b't', b'i', b't', b'l', b'e',
        b'T', b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e', b'a', b'r',
        b'2', b'0', b'2', b'3',
    ];
    let parsed = RawEntryData::from_byte_repr(malformed_data);
    assert!(matches!(
        parsed,
        Err(InvalidBytesError {
            position: 0,
            message: "invalid version"
        })
    ));

    // entry type is not valid utf-8
    let malformed_data = vec![
        0, 7, b'a', b'r', b't', 255, b'c', b'l', b'e', 5, 9, 0, b't', b'i', b't', b'l', b'e', b'T',
        b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e', b'a', b'r', b'2',
        b'0', b'2', b'3',
    ];
    let parsed = RawEntryData::from_byte_repr(malformed_data);
    assert!(matches!(parsed, Err(InvalidBytesError { position: 2, .. })));

    // bad length header
    let malformed_data = vec![
        0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 5, 100, 0, b't', b'i', b't', b'l', b'e',
        b'T', b'h', b'e', b' ', b'T', b'i', b't', b'l', b'e', 4, 4, 0, b'y', b'e', b'a', b'r',
        b'2', b'0', b'2', b'3',
    ];
    let parsed = RawEntryData::from_byte_repr(malformed_data);
    assert!(matches!(
        parsed,
        Err(InvalidBytesError {
            position: 17,
            message: "value block shorter than header"
        })
    ));

    // trailing bytes
    let malformed_data = vec![0, 7, b'a', b'r', b't', b'i', b'c', b'l', b'e', 1];
    let parsed = RawEntryData::from_byte_repr(malformed_data);
    assert!(parsed.is_err());

    // entry type cannot have length 0
    let malformed_data = vec![0, 0];
    let parsed = RawEntryData::from_byte_repr(malformed_data);
    assert!(parsed.is_err());

    // field key cannot have length 0
    let malformed_data = vec![0, 1, b'a', 0, 0, 0];
    let parsed = RawEntryData::from_byte_repr(malformed_data);
    assert!(parsed.is_err());
}

#[test]
fn test_data_err_insert() {
    assert_eq!(
        MutableEntryData::try_new("".into()),
        Err(RecordDataError::EntryTypeInvalidLength(0)),
    );

    assert_eq!(
        MutableEntryData::try_new("b".repeat(300)),
        Err(RecordDataError::EntryTypeInvalidLength(300)),
    );

    assert_eq!(
        MutableEntryData::try_new("🍄".into()),
        Err(RecordDataError::ContainsInvalidChar),
    );

    let mut record_data = MutableEntryData::try_new("a".into()).unwrap();

    assert_eq!(
        record_data.check_and_insert("BAD".into(), "".into()),
        Err(RecordDataError::ContainsInvalidChar)
    );

    assert_eq!(
        record_data.check_and_insert("".into(), "".into()),
        Err(RecordDataError::KeyInvalidLength(0))
    );

    assert!(record_data.is_empty());
}
