#[cfg(feature = "localread")]
pub mod read;

#[cfg(feature = "localwrite")]
pub mod write;

#[cfg(feature = "localread")]
pub use read::LocalReadClient;

#[cfg(feature = "localwrite")]
pub use write::LocalWriteClient;

use std::collections::HashMap;

use bincode::{Decode, Encode};
use ureq::{
    Body,
    http::{
        Response,
        header::{HeaderName, HeaderValue},
    },
};

/// A raw representation of the bytes of a HTTP/1.1 response.
#[derive(Debug, PartialEq, Clone, Decode, Encode)]
pub struct ResponseBytes {
    pub status: u16,
    pub headers: HashMap<Vec<u8>, Vec<u8>>,
    pub body: Vec<u8>,
}

static LOCALPROXY_DATA_FILE: &str = "localproxy.dat";

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
