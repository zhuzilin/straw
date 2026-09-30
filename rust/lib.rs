//! Durable shared-filesystem queue. The Rust core has no Python dependency.
pub mod coordinator;
pub mod gc;
pub mod journal;
pub mod payload;
#[cfg(feature = "python")]
mod python;
pub mod store;

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Debug)]
pub struct Error {
    pub kind: String,
    pub message: String,
}
impl Error {
    pub fn new(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new(
            if matches!(e.raw_os_error(), Some(28 | 122)) {
                "QuotaExceeded"
            } else {
                "StorageUnavailable"
            },
            e.to_string(),
        )
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new("CorruptData", e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub type Fault<'a> = dyn FnMut(&str) -> Result<()> + 'a;
pub fn bytes(value: &Value) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(value)?)
}
pub fn hash(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
pub fn digest(value: &Value) -> Result<String> {
    Ok(hash(&bytes(value)?))
}
pub fn uid() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
pub fn fail<T>(kind: &str, message: impl Into<String>) -> Result<T> {
    Err(Error::new(kind, message))
}
pub fn field<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str()
        .ok_or_else(|| Error::new("InvalidReference", format!("Missing string {k}")))
}
pub fn number(v: &Value, k: &str) -> u64 {
    v[k].as_u64().unwrap_or(0)
}
pub fn array<'a>(v: &'a Value, k: &str) -> Result<&'a Vec<Value>> {
    v[k].as_array()
        .ok_or_else(|| Error::new("InvalidReference", format!("Missing array {k}")))
}
