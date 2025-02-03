use std::fmt;

use serde::{
    de::{self, Deserializer, Error, SeqAccess, Unexpected, Visitor},
    Deserialize,
};

use crate::error::RecordDataError;

use super::{Entry, EntryType, FieldKey, FieldValue, RecordData};

impl<'de> de::Deserialize<'de> for EntryType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut inner = String::deserialize(deserializer)?;
        inner.make_ascii_lowercase();

        Self::try_new(inner).map_err(|err| match err {
            RecordDataError::EntryTypeNotAsciiLowercase => D::Error::invalid_value(
                Unexpected::Other("entry type"),
                &"an entry type with at most 256 ASCII letters",
            ),
            RecordDataError::EntryTypeInvalidLength(_) => D::Error::invalid_value(
                Unexpected::Other("entry type"),
                &"an entry type of at most 256 ASCII letters",
            ),
            _ => unreachable!(),
        })
    }
}

impl<'de> de::Deserialize<'de> for FieldKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut inner = String::deserialize(deserializer)?;
        inner.make_ascii_lowercase();

        Self::try_new(inner).map_err(|err| match err {
            RecordDataError::KeyInvalidLength(_) => D::Error::invalid_value(
                Unexpected::Other("field key"),
                &"a field key with between 1 and 256 ASCII letters",
            ),
            RecordDataError::KeyNotAsciiLowercase => D::Error::invalid_value(
                Unexpected::Other("field key"),
                &"a field key with between 1 and 256 ASCII letters",
            ),
            _ => unreachable!(),
        })
    }
}

impl<'de> de::Deserialize<'de> for FieldValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let inner = String::deserialize(deserializer)?;

        if inner.len() > super::ValueHeader::MAX.into() {
            Err(D::Error::invalid_value(
                Unexpected::Other("field value"),
                &"a field value with size at most 2^16 bytes",
            ))
        } else {
            // SAFETY: we do not check for the 'balanced `{}`' rule here because this rule is
            // automatically checked during bibtex deserialization process, and if we get it wrong,
            // it will not result in data corruption (just invalid output)
            Ok(FieldValue(inner))
        }
    }
}

impl<'de> de::Deserialize<'de> for Entry<RecordData> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum StructField {
            EntryType,
            EntryKey,
            Fields,
        }

        struct EntryVisitor;

        impl<'de> Visitor<'de> for EntryVisitor {
            type Value = Entry<RecordData>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct OwnedEntry")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let entry_type: EntryType<String> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let entry_key: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let fields: std::collections::BTreeMap<FieldKey<String>, FieldValue<String>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Entry {
                    key: serde_bibtex::token::EntryKey::new(entry_key).unwrap(),
                    record_data: RecordData { entry_type, fields },
                })
            }
        }

        deserializer.deserialize_tuple(3, EntryVisitor)
    }
}
