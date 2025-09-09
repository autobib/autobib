use std::collections::HashMap;

use anyhow::{Error, anyhow};
use bincode::{Decode, Encode};
use ureq::{
    Body,
    http::{
        Response,
        header::{HeaderName, HeaderValue},
    },
};

/// A raw representation of the bytes of a HTTP/1.1 response.
#[derive(Decode, Encode)]
pub struct ResponseBytes {
    pub status: u16,
    pub headers: HashMap<Vec<u8>, Vec<u8>>,
    pub body: BodyBytes,
}

/// A raw representation of the body of a HTTP/1.1 response.
#[derive(Decode, Encode)]
pub struct BodyBytes {
    pub contents: Vec<u8>,
    pub mime_type: Option<String>,
    pub charset: Option<String>,
}

impl TryFrom<ResponseBytes> for Response<Body> {
    type Error = Error;

    fn try_from(
        ResponseBytes {
            status,
            headers,
            body,
        }: ResponseBytes,
    ) -> Result<Self, Self::Error> {
        let BodyBytes {
            contents,
            mime_type,
            charset,
        } = body;
        let mut builder = Body::builder();
        builder = if let Some(s) = mime_type {
            builder.mime_type(s)
        } else {
            builder
        };
        builder = if let Some(s) = charset {
            builder.charset(s)
        } else {
            builder
        };
        let body = builder.data(contents);

        let mut res = Response::builder().status(status);
        let h = res
            .headers_mut()
            .ok_or(anyhow!("Failed to construct response headers."))?;
        for (k, v) in headers.iter() {
            let name = HeaderName::from_lowercase(k)?;
            let value = HeaderValue::from_bytes(v)?;
            h.insert(name, value);
        }

        Ok(res.body(body)?)
    }
}

impl From<&mut Body> for BodyBytes {
    fn from(body: &mut Body) -> Self {
        let contents = body.read_to_vec().unwrap();
        let mime_type = body.mime_type().map(ToOwned::to_owned);
        let charset = body.charset().map(ToOwned::to_owned);
        Self {
            contents,
            mime_type,
            charset,
        }
    }
}

impl<B> From<&mut Response<B>> for ResponseBytes
where
    BodyBytes: for<'a> From<&'a mut B>,
{
    fn from(resp: &mut Response<B>) -> Self {
        let status = resp.status().as_u16();
        let mut headers = HashMap::new();
        for (k, v) in resp.headers().iter() {
            headers.insert(k.as_str().to_owned().into(), v.as_ref().to_owned());
        }

        let body = resp.body_mut().into();

        Self {
            status,
            headers,
            body,
        }
    }
}
