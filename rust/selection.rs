//! Preserve the first occurrence of each scheduling key before issuing leases.
use std::collections::BTreeSet;

pub type Key = (i64, i64, usize);

pub fn first_occurrences(keys: &[Key]) -> Vec<bool> {
    let mut seen: BTreeSet<Key> = BTreeSet::new();
    let mut result: Vec<bool> = Vec::new();
    let mut i = 0usize;

    while i < keys.len() {
        result.push(seen.insert(keys[i]));
        i += 1;
    }
    result
}
