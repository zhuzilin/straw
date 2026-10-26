//! Heap-backed frontier. Scheduling deduplicates before expansion, including
//! repeated edges and cycles; the caller supplies immutable node identities.
use std::collections::HashSet;

pub struct Frontier<T> {
    pub seen: HashSet<String>,
    pub pending: Vec<(String, T)>,
}

pub fn start<T>() -> Frontier<T> {
    Frontier {
        seen: HashSet::new(),
        pending: Vec::new(),
    }
}

pub fn schedule<T>(state: &mut Frontier<T>, key: String, value: T) -> bool {
    if !state.seen.insert(key.clone()) {
        return false;
    }
    state.pending.push((key, value));
    true
}

pub fn next<T>(state: &mut Frontier<T>) -> Option<(String, T)> {
    state.pending.pop()
}
