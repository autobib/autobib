use reqwest::{
    blocking::{Client, Response},
    Error, IntoUrl,
};

/// A wrapper around a [`reqwest::blocking::Client`].
pub struct HttpClient {
    client: Client,
}

impl HttpClient {
    pub fn new() -> Result<Self, Error> {
        let client = Client::builder().build()?;
        Ok(HttpClient { client })
    }

    pub fn get<U: IntoUrl>(&self, url: U) -> Result<Response, Error> {
        self.client.get(url).send()
    }
}
