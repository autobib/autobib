use reqwest::{
    blocking::{Client, Response},
    Error, IntoUrl,
};

static APP_USER_AGENT: &str = concat!(
    env!("CARGO_PKG_NAME"),
    "/",
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("CARGO_PKG_HOMEPAGE"),
    "; ",
    env!("CARGO_PKG_AUTHORS"),
    ")",
);

/// A wrapper around a [`reqwest::blocking::Client`].
pub struct HttpClient {
    client: Client,
}

impl HttpClient {
    /// Initialize a new client.
    pub fn new() -> Result<Self, Error> {
        let client = Client::builder().user_agent(APP_USER_AGENT).build()?;
        Ok(HttpClient { client })
    }

    /// Make a request to a given url.
    pub fn get<U: IntoUrl>(&self, url: U) -> Result<Response, Error> {
        self.client.get(url).send()
    }
}
