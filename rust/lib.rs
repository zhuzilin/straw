//! Durable shared-filesystem queue. The Rust core has no Python dependency.
#[cfg(feature = "python")]
mod access;
pub mod bounds;
mod catalog_append;
mod catalog_decode;
mod catalog_event;
mod catalog_frame;
mod catalog_json;
mod catalog_log;
mod catalog_namespace;
mod catalog_primitives;
mod catalog_runtime;
mod catalog_state;
mod catalog_target;
mod catalog_transaction;
mod closure;
mod closure_node;
mod consumption;
pub mod coordinator;
mod directory;
mod directory_io;
mod discovery;
mod file_adapter;
mod file_primitives;
mod finalization;
mod framing;
pub mod gc;
mod gc_batch;
mod gc_collection;
mod gc_io;
mod gc_mark;
mod gc_receipt;
mod holding;
mod identities;
mod incomplete;
pub mod journal;
mod journal_append;
mod journal_io;
mod journal_owner;
mod journal_primitives;
mod journal_recovery;
mod journal_state;
mod json_update;
mod namespace_identity;
mod namespace_step;
mod namespace_target;
pub mod payload;
mod pending;
pub(crate) mod protocol;
#[cfg(feature = "python")]
mod python;
mod queue_admin;
mod queue_application;
mod queue_commit;
mod queue_event;
mod queue_gc;
mod queue_gc_prepare;
mod queue_history;
mod queue_metadata;
mod queue_progress;
mod queue_reader;
mod queue_receipt;
mod queue_replay;
mod queue_retirement;
mod queue_roots;
mod queue_state;
mod queue_task;
mod queue_transition;
mod reclamation;
mod record_manifest;
mod reference_node;
mod reference_validation;
mod replay;
mod request;
pub(crate) mod selection;
mod staging_consumption;
pub mod store;
mod stream;
mod traversal;
mod typed;
mod wal;
mod wal_append;
mod wal_device;
mod wal_file;
mod wal_recovery;
mod wal_sink;
mod wal_source;
mod writer_gate;
mod writer_gate_flow;
mod writer_ownership;
mod writer_registry;
mod writer_retirement;

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
pub fn checked_sum(values: impl IntoIterator<Item = u64>, kind: &str) -> Result<u64> {
    values.into_iter().try_fold(0u64, |total, n| {
        bounds::add(total, n).ok_or_else(|| Error::new(kind, "Unsigned 64-bit accounting overflow"))
    })
}
pub fn array<'a>(v: &'a Value, k: &str) -> Result<&'a Vec<Value>> {
    v[k].as_array()
        .ok_or_else(|| Error::new("InvalidReference", format!("Missing array {k}")))
}

mod record_index;

mod extent_retry;

mod queue_handoff;
