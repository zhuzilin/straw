//! Bounded access to caller-owned record payloads. Logical records may be much
//! larger than the writer's scratch buffer; their on-disk framing is unchanged.
use crate::{Result, store::Record};
use serde_json::Value;
use std::sync::Arc;

pub trait Payload: Send + Sync {
    /// Length captured once for the publication envelope. Each traversal must
    /// return exactly these bytes; changed contents/lengths are rejected.
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn chunks(&self, size: usize, visit: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()>;
}

/// Validate each traversal independently and retain the first callback error
/// even if a custom provider ignores it. Providers must still terminate; this
/// wrapper cannot preempt arbitrary caller code.
pub(crate) fn visit_exact(
    payload: &dyn Payload,
    size: usize,
    expected: u64,
    visit: &mut dyn FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    let mut state = crate::stream::start(expected);
    let mut first_error = None;
    let outcome = payload.chunks(size, &mut |chunk| {
        let result = if crate::stream::accept(&mut state, chunk.len() as u64) {
            visit(chunk)
        } else {
            crate::fail(
                "ValueError",
                "Payload stream exceeds declared length or already failed",
            )
        };
        if let Err(error) = result {
            crate::stream::fail(&mut state);
            let response = crate::Error::new(&error.kind, error.message.clone());
            if first_error.is_none() {
                first_error = Some(error);
            }
            Err(response)
        } else {
            Ok(())
        }
    });
    if let Some(error) = first_error {
        return Err(error);
    }
    outcome?;
    if !crate::stream::complete(&state) {
        return crate::fail(
            "ValueError",
            "Payload stream is shorter than declared length",
        );
    }
    Ok(())
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
