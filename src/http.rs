use std::{fmt::Display, sync::LazyLock};

use reqwest::{
    Error, IntoUrl,
    blocking::{Client as ReqwestClient, Response},
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

static HTTP_CLIENT: LazyLock<ReqwestClient> = LazyLock::new(|| {
    ReqwestClient::builder()
        .user_agent(APP_USER_AGENT)
        .build()
        .unwrap()
});

use crate::logger::info;

pub trait Client: Sized {
    fn new() -> Self;

    fn get<U: IntoUrl + Display>(&self, url: U) -> Result<Response, Error>;
}

pub struct AsyncClient {
    inner: &'static LazyLock<ReqwestClient>,
}

impl Client for AsyncClient {
    fn new() -> Self {
        Self {
            inner: &HTTP_CLIENT,
        }
    }

    fn get<U: IntoUrl + Display>(&self, url: U) -> Result<Response, Error> {
        info!("Making request to url: {url}");
        self.inner.get(url).send()
    }
}
