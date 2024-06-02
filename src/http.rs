use std::thread::sleep;
use std::time::Duration;

use reqwest::{
    blocking::{Client, Response},
    Error, StatusCode,
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
    pub fn get<U: AsRef<str>>(&self, url: U) -> Result<Response, Error> {
        // initial backoff
        let mut backoff = Duration::from_millis(500);

        loop {
            match self.client.get(url.as_ref()).send() {
                Ok(resp) => break Ok(resp),
                Err(err) => {
                    if err.status() == Some(StatusCode::SERVICE_UNAVAILABLE)
                        || err.status() == Some(StatusCode::TOO_MANY_REQUESTS)
                    {
                        sleep(backoff);
                        backoff *= 2;
                    } else {
                        break Err(err);
                    }
                }
            }
        }
    }
}
