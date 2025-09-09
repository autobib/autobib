use std::{fmt::Display, sync::LazyLock};

use bytes::Bytes;
use http::StatusCode;
use reqwest::{
    Error, IntoUrl,
    blocking::{Client as ReqwestClient, Response as ReqwestResponse},
};

use crate::{error::ProviderError, logger::info};

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

static HTTP_CLIENT: LazyLock<ReqwestClient> = LazyLock::new(|| {
    ReqwestClient::builder()
        .user_agent(APP_USER_AGENT)
        .build()
        .unwrap()
});

pub trait Response {
    fn status(&self) -> StatusCode;

    fn bytes(self) -> Result<Bytes, Error>;
}

pub trait Client: Sized {
    type Response: Response;

    fn new() -> Self;

    fn get<U: IntoUrl + Display>(&self, url: U) -> Result<Self::Response, ProviderError>;
}

pub struct AsyncClient {
    inner: &'static LazyLock<ReqwestClient>,
}

impl Response for ReqwestResponse {
    fn status(&self) -> StatusCode {
        Self::status(self)
    }

    fn bytes(self) -> Result<Bytes, Error> {
        Self::bytes(self)
    }
}

impl Client for AsyncClient {
    type Response = ReqwestResponse;

    fn new() -> Self {
        Self {
            inner: &HTTP_CLIENT,
        }
    }

    fn get<U: IntoUrl + Display>(&self, url: U) -> Result<Self::Response, ProviderError> {
        info!("Making request to url: {url}");
        self.inner.get(url).send().map_err(Into::into)
    }
}
