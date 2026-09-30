//! Bounded access to caller-owned record payloads. Logical records may be much
//! larger than the writer's scratch buffer; their on-disk framing is unchanged.
use crate::{Result, store::Record};
use serde_json::Value;
use std::sync::Arc;

pub trait Payload: Send + Sync {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn chunks(&self, size: usize, visit: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()>;
}

impl Payload for Arc<[u8]> {
    fn len(&self) -> usize {
        self.as_ref().len()
    }
    fn chunks(&self, size: usize, visit: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> {
        for chunk in self.as_ref().chunks(size) {
            visit(chunk)?;
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct InputRecord {
    pub metadata: Value,
    pub payload: Arc<dyn Payload>,
}

impl From<Record> for InputRecord {
    fn from(record: Record) -> Self {
        Self {
            metadata: record.metadata,
            payload: Arc::new(record.payload),
        }
    }
}
