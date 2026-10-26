//! Persistent catalog indexes and their ordered replay transitions.
//! Wire decoding and path resolution produce operations; this module applies
//! those operations to the same indexes used by ownership and collection.
use std::collections::{BTreeSet, HashMap, HashSet};

pub struct Indexes {
    pub owners: HashMap<String, BTreeSet<String>>,
    pub packs: HashMap<String, bool>,
    pub deleted: HashSet<String>,
    pub pending_deletions: HashSet<String>,
    pub staging_edges: HashMap<String, Vec<String>>,
    pub consumed_handoffs: HashSet<String>,
}

pub enum Operation {
    Owner(String, BTreeSet<String>),
    Release(String),
    Pack(String, bool),
    Mark(String),
    Reclaim(String),
    Edges(String, Vec<String>),
    Receipts(Vec<String>),
}

impl Indexes {
    pub fn new() -> Self {
        Self {
            owners: HashMap::new(),
            packs: HashMap::new(),
            deleted: HashSet::new(),
            pending_deletions: HashSet::new(),
            staging_edges: HashMap::new(),
            consumed_handoffs: HashSet::new(),
        }
    }

    /// Check the whole ordered update without modifying the live indexes.
    /// Only same-event marks need scratch space; historical indexes are shared.
    pub fn validate(&self, ops: &[Operation]) -> Result<(), usize> {
        let mut marked: HashSet<String> = HashSet::new();
        let mut i = 0usize;

        while i < ops.len() {
            #[allow(clippy::collapsible_match)]
            match &ops[i] {
                Operation::Mark(path) => {
                    marked.insert(path.clone());
                }
                Operation::Reclaim(path) => {
                    if !self.deleted.contains(path) && !marked.contains(path) {
                        return Err(i);
                    }
                }
                _ => {}
            }
            i += 1;
        }

        Ok(())
    }

    pub fn roots(&self) -> HashSet<String> {
        let mut result = HashSet::new();
        let mut entries = self.owners.values();

        loop {
            let Some(paths) = entries.next() else {
                return result;
            };

            extend_roots(&mut result, paths);
        }
    }

    pub fn candidates(&self, live: &HashSet<String>) -> HashSet<String> {
        let mut result = HashSet::new();
        let mut entries = self.packs.iter();

        loop {
            let Some((path, sealed)) = entries.next() else {
                return result;
            };

            let status = crate::protocol::PackStatus {
                sealed: *sealed,
                owned: live.contains(path),
                tombstoned: self.deleted.contains(path),
            };
            if crate::protocol::candidate(status) {
                result.insert(path.clone());
            }
        }
    }

    pub fn apply_one(&mut self, op: &Operation) -> bool {
        match op {
            Operation::Owner(key, paths) => {
                self.owners.insert(key.clone(), paths.clone());
            }
            Operation::Release(key) => {
                self.owners.remove(key);
            }
            Operation::Pack(path, sealed) => {
                self.packs.insert(path.clone(), *sealed);
            }
            Operation::Mark(path) => {
                crate::reclamation::mark(
                    &mut self.deleted,
                    &mut self.pending_deletions,
                    path.clone(),
                );
            }
            Operation::Reclaim(path) => {
                if !crate::reclamation::retire(&self.deleted, &mut self.pending_deletions, path) {
                    return false;
                }
                self.packs.remove(path);
            }
            Operation::Edges(key, children) => {
                self.staging_edges.insert(key.clone(), children.clone());
            }
            Operation::Receipts(ids) => {
                crate::consumption::record_receipts(&mut self.consumed_handoffs, ids);
            }
        }

        true
    }

    pub fn apply(&mut self, ops: &[Operation]) -> Result<(), usize> {
        let mut i = 0usize;

        while i < ops.len() {
            if !self.apply_one(&ops[i]) {
                return Err(i);
            }
            i += 1;
        }

        Ok(())
    }
}

fn extend_roots(result: &mut HashSet<String>, paths: &BTreeSet<String>) {
    let mut entries = paths.iter();

    loop {
        let Some(path) = entries.next() else {
            return;
        };
        result.insert(path.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_uses_only_preceding_marks_and_leaves_indexes_unchanged() {
        let mut indexes = Indexes::new();
        let path = "raw/pack".to_string();
        let owner = Operation::Owner("reader".into(), BTreeSet::from([path.clone()]));
        let invalid = [
            owner,
            Operation::Reclaim(path.clone()),
            Operation::Mark(path.clone()),
        ];
        assert_eq!(indexes.validate(&invalid), Err(1));
        assert!(indexes.owners.is_empty());
        assert!(indexes.deleted.is_empty());
        assert!(indexes.pending_deletions.is_empty());
        let valid = [
            Operation::Mark(path.clone()),
            Operation::Reclaim(path.clone()),
        ];
        assert_eq!(indexes.validate(&valid), Ok(()));
        assert!(indexes.deleted.is_empty());
        assert_eq!(indexes.apply(&valid), Ok(()));
        assert!(indexes.deleted.contains(&path));
        assert!(indexes.pending_deletions.is_empty());
        // Durable old marks remain valid even when no mark occurs in this event.
        assert_eq!(indexes.validate(&[Operation::Reclaim(path)]), Ok(()));
    }
}
