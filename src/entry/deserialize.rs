use std::fmt;

use serde::de::{self, Deserializer, Error, SeqAccess, Unexpected, Visitor};

use super::{
    Entry, EntryKey, EntryType, EntryTypeHeader, FieldKey, FieldValue, KeyHeader, RecordData,
};

impl<'de> de::Deserialize<'de> for EntryType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut inner = String::deserialize(deserializer)?;

        if !inner.is_ascii() {
            return Err(D::Error::invalid_value(
                Unexpected::Str(&inner),
                &"an entry type composed of ASCII characters",
            ));
        }

        if inner.len() > EntryTypeHeader::MAX as usize {
            return Err(D::Error::invalid_value(
                Unexpected::Str(&inner),
                &"an entry type with at most 256 ASCII characters",
            ));
        }

        inner.make_ascii_lowercase();

        // SAFETY: `inner` is only accepted by the serde_bibtex deserialize impl if either it is
        // composed of non-ASCII characters, or ASCII characters which satisfy the field key rules
        // or also possibly capitals `A..=Z`. Therefore we only need to check that it is ASCII, and
        // convert any possible capitals to ASCII lowercase.
        Ok(Self(inner))
    }
}

impl<'de> de::Deserialize<'de> for FieldKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut inner = String::deserialize(deserializer)?;

        if !inner.is_ascii() {
            return Err(D::Error::invalid_value(
                Unexpected::Str(&inner),
                &"a field key composed of ASCII characters",
            ));
        }

        if inner.len() > KeyHeader::MAX as usize {
            return Err(D::Error::invalid_value(
                Unexpected::Str(&inner),
                &"a field key with at most 256 ASCII characters",
            ));
        }

        inner.make_ascii_lowercase();

        // SAFETY: `inner` is only accepted by the serde_bibtex deserialize impl if either it is
        // composed of non-ASCII characters, or ASCII characters which satisfy the field key rules
        // or also possibly capitals `A..=Z`. Therefore we only need to check that it is ASCII, and
        // convert any possible capitals to ASCII lowercase.
        Ok(Self(inner))
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
            Ok(Self(inner))
        }
    }
}

impl<'de> de::Deserialize<'de> for Entry<RecordData> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
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
                    key: EntryKey(entry_key), // SAFETY: serde_bibtex only returns keys satisfying
                    // the requiremens
                    record_data: RecordData { entry_type, fields },
                })
            }
        }

        deserializer.deserialize_tuple(3, EntryVisitor)
    }
}
