//! Account search over WhatsMyName and Sherlock site lists.
//! The library is dependency-free. HTTP goes through the system `curl`.

#![forbid(unsafe_code)]

pub mod catalog;
pub mod detect;
pub mod export;
pub mod http;
pub mod json;
pub mod query;
pub mod re;
pub mod scan;
pub mod sha256;
