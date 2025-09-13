//! # Cache binary format definition
//!
//! This file is hashed by tests scripts and CI so that the cache breaks if the definition changes.
//! Do not add other things to this file, or modify it unnecessarily.
use std::collections::HashMap;

use bincode::{Decode, Encode};

/// A raw representation of the bytes of a HTTP/1.1 response.
#[derive(Debug, PartialEq, Clone, Decode, Encode)]
pub struct ResponseBytes {
    pub status: u16,
    pub headers: HashMap<Vec<u8>, Vec<u8>>,
    pub body: Vec<u8>,
}
