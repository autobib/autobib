#[cfg(any(feature = "localread", feature = "localwrite"))]
pub mod localproxy;

use ureq::{
    Agent, Body,
    http::{self, Uri},
};

use crate::error::ProviderError;

pub trait Client: Sized {
    type Body: BodyBytes;

    fn get<T>(&self, uri: T) -> Result<http::Response<Self::Body>, ProviderError>
    where
        Uri: TryFrom<T>,
        <Uri as TryFrom<T>>::Error: Into<http::Error>;
}

pub trait BodyBytes {
    fn bytes(self) -> Result<Vec<u8>, ProviderError>;
}

impl BodyBytes for Body {
    fn bytes(mut self) -> Result<Vec<u8>, ProviderError> {
        self.read_to_vec().map_err(Into::into)
    }
}

impl BodyBytes for Vec<u8> {
    fn bytes(self) -> Result<Vec<u8>, ProviderError> {
        Ok(self)
    }
}

#[cfg_attr(feature = "localread", allow(unused))]
pub struct UreqClient {
    inner: Agent,
}

#[cfg_attr(feature = "localread", allow(unused))]
impl UreqClient {
    pub fn new() -> Self {
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

        let config = Agent::config_builder()
            .user_agent(APP_USER_AGENT)
            .https_only(true)
            .http_status_as_error(false)
            .build();
        let inner = Agent::new_with_config(config);
        Self { inner }
    }
}

impl Client for UreqClient {
    type Body = Body;

    fn get<T>(&self, uri: T) -> Result<http::Response<Body>, ProviderError>
    where
        Uri: TryFrom<T>,
        <Uri as TryFrom<T>>::Error: Into<http::Error>,
    {
        self.inner.get(uri).call().map_err(Into::into)
    }
}
