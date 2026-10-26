//! Semantic checks on decoded catalog fields, before namespace resolution.
use crate::catalog_json::Wire;
use std::collections::HashMap;

#[allow(clippy::manual_range_contains)]
fn valid_hex(bytes: &[u8], start: usize) -> bool {
    if start > bytes.len() || bytes.len() - start != 64 {
        return false;
    }
    let mut i = start;

    while i < bytes.len() {
        let byte = bytes[i];
        if !((b'0' <= byte && byte <= b'9') || (b'a' <= byte && byte <= b'f')) {
            return false;
        }
        i += 1;
    }
    true
}

pub fn valid_handoff_id(id: &str) -> bool {
    valid_hex(id.as_bytes(), 0)
}

pub fn valid_staged_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    let prefix = [115u8, 116, 97, 103, 101, 100, 58];
    crate::framing::matches_at(bytes, 0, &prefix) && valid_hex(bytes, 7)
}

pub fn valid_receipts(wire: &Wire) -> bool {
    if !wire.has_receipts {
        return true;
    }
    if !wire.has_owners {
        return false;
    }
    let mut i = 0usize;

    while i < wire.receipts.len() {
        if !valid_handoff_id(&wire.receipts[i]) {
            return false;
        }
        i += 1;
    }
    let mut i = 0usize;

    while i < wire.owners.len() {
        if wire.owners[i].1.is_some() {
            return false;
        }
        i += 1;
    }
    true
}

fn valid_child_names(children: &[String]) -> bool {
    let mut i = 0usize;

    while i < children.len() {
        if !valid_staged_key(&children[i]) {
            return false;
        }
        i += 1;
    }
    true
}

pub fn valid_staging_edges(
    known: &HashMap<String, Vec<String>>,
    additions: &HashMap<String, Vec<String>>,
) -> bool {
    let mut entries = additions.iter();

    loop {
        let Some((key, children)) = entries.next() else {
            return crate::consumption::compatible(known, additions);
        };

        if !valid_staged_key(key) || !valid_child_names(children) {
            return false;
        }
    }
}
