//! A persisted staging graph must contain every referenced node. The caller
//! binds these keys/edges to authenticated immutable manifests and one frame.
use std::collections::{HashMap, HashSet};

pub fn same_children(left: &[String], right: &[String]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut i = 0usize;

    while i < left.len() {
        if !String::eq(&left[i], &right[i]) {
            return false;
        }
        i += 1;
    }

    true
}

/// Check the whole update graph before applying any of its edges. Every child
/// must have a definition, and stored immutable edges cannot be changed.
pub fn compatible(
    known: &HashMap<String, Vec<String>>,
    additions: &HashMap<String, Vec<String>>,
) -> bool {
    let mut entries = additions.iter();

    loop {
        let Some((key, children)) = entries.next() else {
            return true;
        };

        if !covered(known, additions, children) {
            return false;
        }
        let consistent = match known.get(key) {
            Some(previous) => same_children(previous, children),
            None => true,
        };
        if !consistent {
            return false;
        }
    }
}

// An error may follow the durable receipt. Keep the exact outstanding work
// until the catalog confirms it; a later invocation can recognize that receipt.

pub fn pop_completed<T>(pending: &mut Vec<T>, confirmed: bool) {
    if confirmed {
        pending.pop();
    }
}

pub fn record_receipts(known: &mut HashSet<String>, receipts: &[String]) {
    let mut i = 0usize;

    while i < receipts.len() {
        known.insert(receipts[i].clone());
        i += 1;
    }
}

pub fn covered(
    known: &HashMap<String, Vec<String>>,
    additions: &HashMap<String, Vec<String>>,
    children: &[String],
) -> bool {
    let mut i = 0usize;

    while i < children.len() {
        if !known.contains_key(&children[i]) && !additions.contains_key(&children[i]) {
            return false;
        }
        i += 1;
    }
    true
}
