use serde::Deserialize;
use serde_bibtex::de::Deserializer;

use crate::RecordId;

#[derive(Debug, Deserialize, PartialEq)]
struct OnlyEntryKey<'r> {
    entry_key: &'r str,
}

/// Get citekeys from buffer which is formatted as a bibtex file.
pub fn get_citekeys<T: Extend<RecordId>>(buffer: &[u8], container: &mut T) {
    // iterate over all entries, only extracting the entry key and converting the entry key into a
    // `RecordId`
    let citation_key_iter = Deserializer::from_slice(buffer)
        .into_iter_regular_entry()
        .filter_map(|res| {
            res.ok()
                .map(|OnlyEntryKey { entry_key }| RecordId::from(entry_key))
        });

    container.extend(citation_key_iter);
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_find_bib_citekeys() {
        let input = br#"
@article{key,
   author = {Author}
}
@book{local:1234,}
        "#;
        let mut vec: Vec<RecordId> = Vec::new();
        get_citekeys(input, &mut vec);
        assert!(vec.len() == 2);
        for s in ["key", "local:1234"] {
            assert!(vec.contains(&RecordId::from(s)));
        }
    }
}
