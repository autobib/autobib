use std::fmt::Display;

use reqwest::{
    Error, IntoUrl,
    blocking::{Client as ReqwestClient, Response},
};

use crate::logger::info;

pub trait Client: Sized {
    fn new() -> Result<Self, Error>;

    fn get<U: IntoUrl + Display>(&self, url: U) -> Result<Response, Error>;
}

impl Client for ReqwestClient {
    fn new() -> Result<Self, Error> {
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
        Self::builder().user_agent(APP_USER_AGENT).build()
    }

    fn get<U: IntoUrl + Display>(&self, url: U) -> Result<Response, Error> {
        info!("Making request to url: {url}");
        self.get(url).send()
    }
}
