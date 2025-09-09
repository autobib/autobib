#[cfg(any(feature = "localproxy", test))]
mod localproxy;

use ureq::{
    Agent, Body,
    http::{self, Uri},
};

use crate::error::ProviderError;

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

pub trait Client: Sized {
    fn new() -> Self;

    fn get<T>(&self, uri: T) -> Result<http::Response<Body>, ProviderError>
    where
        Uri: TryFrom<T>,
        <Uri as TryFrom<T>>::Error: Into<http::Error>;
}

pub trait Response {
    fn bytes(&mut self) -> Result<Vec<u8>, ProviderError>;
}

impl Response for http::Response<Body> {
    fn bytes(&mut self) -> Result<Vec<u8>, ProviderError> {
        self.body_mut().read_to_vec().map_err(Into::into)
    }
}

pub struct UreqClient {
    inner: Agent,
}

impl Client for UreqClient {
    fn new() -> Self {
        let config = Agent::config_builder()
            .user_agent(APP_USER_AGENT)
            .https_only(true)
            .http_status_as_error(false)
            .build();
        let inner = Agent::new_with_config(config);
        Self { inner }
    }

    fn get<T>(&self, uri: T) -> Result<http::Response<Body>, ProviderError>
    where
        Uri: TryFrom<T>,
        <Uri as TryFrom<T>>::Error: Into<http::Error>,
    {
        self.inner.get(uri).call().map_err(Into::into)
    }
}
