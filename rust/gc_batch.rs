//! The production batch deletion dispatch. Native effects must bind each index
//! to its durable tombstone and stable parent directory; errors may have effects.
use crate::protocol::{self, DeleteEffect, ReclaimEffect};
use std::collections::BTreeSet;

pub trait BatchModel {
    type Parent: Ord;
}

pub struct Removal<P> {
    pub parent: P,
    /// Some means unlink removed a name; None is an unlink NotFound response.
    pub bytes: Option<u64>,
}

pub struct Statistics {
    pub files: u64,
    pub bytes: u64,
}

pub enum Failure<E> {
    Effect(E),
    Protocol,
    Accounting,
}

pub trait Effects: BatchModel {
    type Error;

    fn target_count(&self) -> usize;

    fn marked(&self, index: usize) -> bool;

    fn unlink(&mut self, index: usize) -> Result<Removal<Self::Parent>, Self::Error>;

    fn sync_parent(&mut self, parent: &Self::Parent) -> Result<(), Self::Error>;

    fn receipt(&mut self) -> Result<(), Self::Error>;
}

/// Deduplicate parent barriers only after every target has been unlinked.
/// Already-absent names still contribute their parent; a receipt may survive
/// an error response, so its precondition requires durable absence already.
pub fn collect<E: Effects>(effects: &mut E) -> Result<Statistics, Failure<E::Error>> {
    let count = effects.target_count();
    let mut parents = BTreeSet::new();
    let mut pending = Vec::new();
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut index = 0usize;

    while index < count {
        let marked = effects.marked(index);
        let deletion = protocol::delete_start(marked);
        if !matches!(protocol::delete_next(&deletion), Some(DeleteEffect::Unlink)) {
            return Err(Failure::Protocol);
        }
        let removal = match effects.unlink(index) {
            Ok(removal) => removal,
            Err(error) => return Err(Failure::Effect(error)),
        };
        let deletion = protocol::delete_advance(deletion, true);
        parents.insert(removal.parent);
        pending.push(deletion);
        if let Some(size) = removal.bytes {
            files = match crate::bounds::add(files, 1) {
                Some(value) => value,
                None => return Err(Failure::Accounting),
            };
            bytes = match crate::bounds::add(bytes, size) {
                Some(value) => value,
                None => return Err(Failure::Accounting),
            };
        }
        index += 1;
    }

    let mut state = protocol::reclaim_start();

    loop {
        let effect = match protocol::reclaim_next(&state) {
            Some(effect) => effect,
            None => {
                if !protocol::reclaim_finished(&state) {
                    return Err(Failure::Protocol);
                }
                return Ok(Statistics { files, bytes });
            }
        };
        match effect {
            ReclaimEffect::SyncParents => {
                for parent in &parents {
                    effects.sync_parent(parent).map_err(Failure::Effect)?;
                }

                if !protocol::delete_batch_finished(&pending, true) {
                    return Err(Failure::Protocol);
                }
            }
            ReclaimEffect::RecordReceipt => {
                if let Err(error) = effects.receipt() {
                    return Err(Failure::Effect(error));
                }
            }
        }
        state = protocol::reclaim_advance(state, true);
    }
}
