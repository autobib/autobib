use std::str::from_utf8;

use autobib_entry::{Archive, v1::ArchivedEntryData};
use chrono::{DateTime, Local};
use rusqlite::{Row, types::ValueRef};

use crate::{
    db::{
        Record,
        state::{
            ArbitraryData, ArbitraryDataRef, HistRecord, NotVoidData, RevId, TxRevId, Variant,
            WithRev,
        },
    },
    record::Key,
    record::{Identifier, KeyedRecord},
};

use super::{AccessRow, AccessRowUnchecked, col};

// core types

/// Read a [`RevId`] from a `rev` column.
impl<'row> AccessRowUnchecked<'row> for RevId {
    fn access_row_unchecked(row: &'row Row<'_>) -> Self {
        Self(row.get_unwrap("rev"))
    }
}
impl<'row, Q: col::Rev> AccessRow<'row, Q> for RevId {}

impl<'row> AccessRowUnchecked<'row> for TxRevId {
    fn access_row_unchecked(row: &'row Row<'_>) -> Self {
        Self::new(row.get_unwrap("rev"))
    }
}
impl<'row, Q: col::Rev> AccessRow<'row, Q> for TxRevId {}

impl<'row> AccessRowUnchecked<'row> for Variant {
    fn access_row_unchecked(row: &'row Row<'_>) -> Self {
        let ValueRef::Integer(variant) = row.get_ref_unwrap("variant") else {
            panic!("Expected 'variant' column to be of type INTEGER");
        };
        match variant {
            0 => Self::Entry,
            1 => Self::Deleted,
            2 => Self::Void,
            _ => panic!(
                "Unexpected 'Records' table record variant: expected 0 (entry), 1 (deleted), or 2 (void)."
            ),
        }
    }
}
impl<'row, Q: col::Variant> AccessRow<'row, Q> for Variant {}

impl<'row> AccessRowUnchecked<'row> for Key {
    fn access_row_unchecked(row: &'row Row<'_>) -> Self {
        String::access_row_unchecked(row).into()
    }
}
impl<'row, Q: col::Name> AccessRow<'row, Q> for Key {}

impl<'row> AccessRowUnchecked<'row> for &'row str {
    fn access_row_unchecked(row: &'row Row<'_>) -> Self {
        let ValueRef::Text(name) = row.get_ref_unwrap("name") else {
            panic!("Expected 'name' column to be of type TEXT");
        };
        from_utf8(name).unwrap()
    }
}
impl<'row, Q: col::Name> AccessRow<'row, Q> for &'row str {}

impl<'row> AccessRowUnchecked<'row> for String {
    fn access_row_unchecked(row: &'row Row<'_>) -> Self {
        <&str>::access_row_unchecked(row).to_owned()
    }
}
impl<'row, Q: col::Name> AccessRow<'row, Q> for String {}

impl<'row> AccessRowUnchecked<'row> for Option<TxRevId> {
    fn access_row_unchecked(row: &'row Row<'_>) -> Self {
        row.get_unwrap::<_, Option<i64>>("parent_rev")
            .map(TxRevId::new)
    }
}
impl<'row, Q: col::Parent> AccessRow<'row, Q> for Option<TxRevId> {}

impl<'row> AccessRowUnchecked<'row> for Identifier<&'row str> {
    fn access_row_unchecked(row: &'row Row<'_>) -> Self {
        let ValueRef::Text(canonical) = row.get_ref_unwrap("canonical") else {
            panic!("Expected 'canonical' column to be of type TEXT");
        };
        Identifier::from_string_unchecked(from_utf8(canonical).unwrap())
    }
}
impl<'row, Q: col::Canonical> AccessRow<'row, Q> for Identifier<&'row str> {}

impl<'row> AccessRowUnchecked<'row> for Identifier {
    fn access_row_unchecked(row: &'row Row<'_>) -> Self {
        Identifier::<&'row str>::access_row_unchecked(row).as_owned()
    }
}
impl<'row, Q: col::Canonical> AccessRow<'row, Q> for Identifier {}

impl<'row> AccessRowUnchecked<'row> for DateTime<Local> {
    fn access_row_unchecked(row: &'row Row<'_>) -> Self {
        row.get_unwrap("modified")
    }
}
impl<'row, Q: col::Modified> AccessRow<'row, Q> for DateTime<Local> {}

impl<'row> AccessRowUnchecked<'row> for () {
    fn access_row_unchecked(_: &'row Row<'_>) {}
}
impl<'row, Q: col::DataVoid> AccessRow<'row, Q> for () {}

impl<'row> AccessRowUnchecked<'row> for Option<Identifier<&'row str>> {
    fn access_row_unchecked(row: &'row Row<'_>) -> Self {
        let ValueRef::Blob(raw) = row.get_ref_unwrap("data") else {
            panic!("Expected 'data' column to be of type BLOB");
        };
        if raw.is_empty() {
            None
        } else {
            Some(Identifier::from_string_unchecked(from_utf8(raw).unwrap()))
        }
    }
}
impl<'row, Q: col::DataDeleted> AccessRow<'row, Q> for Option<Identifier<&'row str>> {}

impl<'row> AccessRowUnchecked<'row> for Option<Identifier> {
    fn access_row_unchecked(row: &'row Row<'_>) -> Self {
        Option::<Identifier<&'row str>>::access_row_unchecked(row).map(|i| i.as_owned())
    }
}
impl<'row, Q: col::DataDeleted> AccessRow<'row, Q> for Option<Identifier> {}

impl<'r> AccessRowUnchecked<'r> for &'r ArchivedEntryData {
    fn access_row_unchecked(row: &'r Row<'_>) -> Self {
        let ValueRef::Blob(data_bytes) = row.get_ref_unwrap("data") else {
            panic!("Expected 'data' column to be of type BLOB");
        };
        ArchivedEntryData::access(data_bytes)
            .expect("Database contains malformed binary data in the 'data' column.")
    }
}
impl<'r, Q: col::DataEntry> AccessRow<'r, Q> for &'r ArchivedEntryData {}

impl<'r> AccessRowUnchecked<'r> for Box<ArchivedEntryData> {
    fn access_row_unchecked(row: &'r Row<'_>) -> Self {
        <&'r ArchivedEntryData>::access_row_unchecked(row).to_owned()
    }
}
impl<'r, Q: col::DataEntry> AccessRow<'r, Q> for Box<ArchivedEntryData> {}

impl<'r> AccessRowUnchecked<'r> for ArbitraryDataRef<'r> {
    fn access_row_unchecked(row: &'r Row<'_>) -> Self {
        match Variant::access_row_unchecked(row) {
            Variant::Entry => Self::Entry(<&ArchivedEntryData>::access_row_unchecked(row)),
            Variant::Deleted => {
                Self::Deleted(Option::<Identifier<&str>>::access_row_unchecked(row))
            }
            Variant::Void => Self::Void,
        }
    }
}
impl<'r, Q: col::DataArbitrary> AccessRow<'r, Q> for ArbitraryDataRef<'r> {}

impl<'r> AccessRowUnchecked<'r> for ArbitraryData {
    fn access_row_unchecked(row: &'r Row<'_>) -> Self {
        ArbitraryDataRef::access_row_unchecked(row).as_owned()
    }
}
impl<'r, Q: col::DataArbitrary> AccessRow<'r, Q> for ArbitraryData {}

impl<'r> AccessRowUnchecked<'r> for NotVoidData {
    fn access_row_unchecked(row: &'r Row<'_>) -> Self {
        match Variant::access_row_unchecked(row) {
            Variant::Entry => Self::Entry(Box::access_row_unchecked(row)),
            Variant::Deleted => Self::Deleted(Option::access_row_unchecked(row)),
            Variant::Void => panic!(
                "Expected data variant: 0 (entry) or 1 (deleted), got 2 (void).\nThis is likely a bug in Autobib, please report it."
            ),
        }
    }
}
impl<'r, Q: col::DataNotVoid> AccessRow<'r, Q> for NotVoidData {}

impl<'r, D, S> AccessRowUnchecked<'r> for Record<D, S>
where
    D: AccessRowUnchecked<'r>,
    Identifier<S>: AccessRowUnchecked<'r>,
{
    fn access_row_unchecked(row: &'r Row<'_>) -> Self {
        let data = D::access_row_unchecked(row);
        let canonical = Identifier::access_row_unchecked(row);
        let modified = DateTime::<Local>::access_row_unchecked(row);

        Self {
            data,
            modified,
            canonical,
        }
    }
}
impl<'r, D, S, Q: col::Modified> AccessRow<'r, Q> for Record<D, S>
where
    D: AccessRow<'r, Q>,
    Identifier<S>: AccessRow<'r, Q>,
{
}

impl<'row, D, S> AccessRowUnchecked<'row> for HistRecord<D, S>
where
    Record<D, S>: AccessRowUnchecked<'row>,
{
    fn access_row_unchecked(row: &'row Row<'_>) -> Self {
        let parent = Option::<TxRevId>::access_row_unchecked(row);
        let record = Record::access_row_unchecked(row);

        Self { record, parent }
    }
}
impl<'row, D, S, Q: col::Parent> AccessRow<'row, Q> for HistRecord<D, S> where
    Record<D, S>: AccessRow<'row, Q>
{
}

impl<'r, R, K> AccessRowUnchecked<'r> for KeyedRecord<R, K>
where
    R: AccessRowUnchecked<'r>,
    K: AccessRowUnchecked<'r>,
{
    fn access_row_unchecked(row: &'r Row<'_>) -> Self {
        let record = R::access_row_unchecked(row);
        let key = K::access_row_unchecked(row);
        Self { key, record }
    }
}
impl<'r, R, K, Q: col::Name> AccessRow<'r, Q> for KeyedRecord<R, K>
where
    R: AccessRow<'r, Q>,
    K: AccessRow<'r, Q>,
{
}

impl<'r, R, V> AccessRowUnchecked<'r> for WithRev<R, V>
where
    R: AccessRowUnchecked<'r>,
    V: AccessRowUnchecked<'r>,
{
    fn access_row_unchecked(row: &'r Row<'_>) -> Self {
        let inner = R::access_row_unchecked(row);
        let rev = V::access_row_unchecked(row);
        Self { inner, rev }
    }
}
impl<'r, R, V, Q: col::Rev> AccessRow<'r, Q> for WithRev<R, V>
where
    R: AccessRow<'r, Q>,
    V: AccessRow<'r, Q>,
{
}
