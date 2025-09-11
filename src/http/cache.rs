#[cfg(feature = "read_response_cache")]
pub mod read;

#[cfg(all(feature = "write_response_cache", not(feature = "read_response_cache")))]
pub mod write;

use std::{borrow::Cow, collections::HashMap, env::VarError, path::Path};

use bincode::{Decode, Encode};
use ureq::{
    Body,
    http::{
        Response,
        header::{HeaderName, HeaderValue},
    },
};

use crate::logger::warn;
#[cfg(feature = "read_response_cache")]
pub use read::LocalReadClient;
#[cfg(all(feature = "write_response_cache", not(feature = "read_response_cache")))]
pub use write::LocalWriteClient;

/// A raw representation of the bytes of a HTTP/1.1 response.
#[derive(Debug, PartialEq, Clone, Decode, Encode)]
pub struct ResponseBytes {
    pub status: u16,
    pub headers: HashMap<Vec<u8>, Vec<u8>>,
    pub body: Vec<u8>,
}

static DEFAULT_RESPONSE_CACHE_FILE: &str = "response.dat";

fn response_cache_file() -> Cow<'static, Path> {
    match std::env::var("AUTOBIB_RESPONSE_CACHE_FILE") {
        Ok(s) => return Cow::Owned(s.into()),
        Err(VarError::NotPresent) => {}
        Err(VarError::NotUnicode(_)) => {
            warn!(
                "Variable 'AUTOBIB_RESPONSE_CACHE_FILE' is not Unicode. Falling back to default value: '{DEFAULT_RESPONSE_CACHE_FILE}'"
            );
        }
    }
    Cow::Borrowed(Path::new(DEFAULT_RESPONSE_CACHE_FILE))
}

impl TryFrom<&ResponseBytes> for Response<Body> {
    type Error = ureq::Error;

    fn try_from(
        ResponseBytes {
            status,
            headers,
            body,
        }: &ResponseBytes,
    ) -> Result<Self, Self::Error> {
        let builder = Body::builder().data(body.clone());

        let mut res = Response::builder().status(*status);
        let h = res.headers_mut().ok_or(ureq::Error::Other(
            "Failed to construct response headers.".to_string().into(),
        ))?;
        for (k, v) in headers.iter() {
            let name =
                HeaderName::from_lowercase(k).map_err(|err| ureq::Error::Other(Box::new(err)))?;
            let value =
                HeaderValue::from_bytes(v).map_err(|err| ureq::Error::Other(Box::new(err)))?;
            h.insert(name, value);
        }

        Ok(res.body(builder)?)
    }
}

impl From<&Response<Vec<u8>>> for ResponseBytes {
    fn from(resp: &Response<Vec<u8>>) -> Self {
        let status = resp.status().as_u16();
        let mut headers = HashMap::new();
        for (k, v) in resp.headers().iter() {
            headers.insert(k.as_str().to_owned().into(), v.as_ref().to_owned());
        }

        let body = resp.body().clone();

        Self {
            status,
            headers,
            body,
        }
    }
}
