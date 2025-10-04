use std::{
    collections::HashMap,
    fs::File,
    sync::{Arc, Mutex},
};

use bincode::config;
use ureq::http::Uri;

use super::{ResponseBytes, response_cache_file};
use crate::http::{Client, UreqClient};

/// A client which intercepts HTTP responses and writes them to a response cache file for
/// subsequent use by `LocalReadClient`.
pub struct LocalWriteClient {
    lookup: Arc<Mutex<HashMap<String, ResponseBytes>>>,
    inner: UreqClient,
}

impl LocalWriteClient {
    pub fn new() -> Self {
        Self {
            lookup: Arc::new(Mutex::new(HashMap::new())),
            inner: UreqClient::new(),
        }
    }
}

impl LocalWriteClient {
    pub fn serialize(self) {
        let data_file = response_cache_file();
        let mut lookup_file = File::create(&data_file).unwrap_or_else(|_| {
            panic!(
                "Failed to create response cache file '{}'",
                data_file.display()
            )
        });

        bincode::encode_into_std_write(&self.lookup, &mut lookup_file, config::standard())
            .expect("Failed to encode lookup table");
    }
}

impl Client for LocalWriteClient {
    type Body = Vec<u8>;

    fn get<T>(&self, uri: T) -> Result<ureq::http::Response<Self::Body>, ureq::Error>
    where
        Uri: TryFrom<T>,
        <Uri as TryFrom<T>>::Error: Into<ureq::http::Error>,
    {
        let uri = Uri::try_from(uri).map_err(Into::into)?;
        let key = uri.to_string();
        let res = self
            .inner
            .get::<Uri>(uri)?
            .map(|mut body| body.read_to_vec().expect("Failed to read request body!"));

        let mut lookup = self.lookup.lock().unwrap();
        lookup.insert(key, ResponseBytes::from(&res));

        Ok(res)
    }
}
