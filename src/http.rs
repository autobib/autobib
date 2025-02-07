use std::fmt::Display;

use reqwest::{
    blocking::{Client, ClientBuilder, Response},
    Error, IntoUrl,
};

use crate::logger::info;

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

/// A thin wrapper around a [`reqwest::blocking::Client`].
pub struct HttpClient {
    client: Client,
}

impl HttpClient {
    /// Initialize a default builder.
    pub fn default_builder() -> ClientBuilder {
        Client::builder().user_agent(APP_USER_AGENT)
    }

    /// Create a new client from the builder.
    pub fn new(builder: ClientBuilder) -> Result<Self, Error> {
        Ok(Self {
            client: builder.build()?,
        })
    }
    /// Make a request to a given url.
    pub fn get<U: IntoUrl + Display>(&self, url: U) -> Result<Response, Error> {
        info!("Making request to url: {url}");
        self.client.get(url).send()
    }
}
