//! # Abstractions over HTTP requests
//!
//! This module provides the [`Client`] trait, which is an abstraction over an HTTP client which
//! can convert URIs into HTTP Response data.

#[cfg(any(feature = "read_response_cache", feature = "write_response_cache"))]
pub mod cache;

use ureq::{
    Body,
    http::{self, Uri},
};

use crate::error::ProviderError;

/// Abstraction over a HTTP client.
pub trait Client {
    /// An HTTP response body which can be efficiently converted into raw bytes.
    type Body: BodyBytes;

    /// Returns the HTTP/1.1 response obtained by a `GET` request to the provided URI.
    fn get<T>(&self, uri: T) -> Result<http::Response<Self::Body>, ProviderError>
    where
        Uri: TryFrom<T>,
        <Uri as TryFrom<T>>::Error: Into<http::Error>;
}

pub trait BodyBytes {
    /// Convert the response body into raw bytes.
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

/// The standard HTTP client, which makes genuine HTTP/1.1 requests using an internal
/// [`ureq::Agent`].
#[cfg(not(feature = "read_response_cache"))]
pub struct UreqClient {
    inner: ureq::Agent,
}

#[cfg(not(feature = "read_response_cache"))]
impl UreqClient {
    /// Construct a new HTTP client with default configuration and correct user agent.
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

        let config = ureq::Agent::config_builder()
            .user_agent(APP_USER_AGENT)
            .https_only(true)
            .http_status_as_error(false)
            .build();
        let inner = ureq::Agent::new_with_config(config);
        Self { inner }
    }
}

#[cfg(not(feature = "read_response_cache"))]
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
