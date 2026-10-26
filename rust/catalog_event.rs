//! Semantic checks on decoded catalog fields, before namespace resolution.
use crate::catalog_json::Wire;
use std::collections::HashMap;
#[cfg(straw_verus)]
use vstd::{prelude::*, std_specs::iter::IteratorSpec, string::StringSliceAdditionalSpecFns};

#[cfg(straw_verus)]
verus! {
    pub open spec fn hex_digit(byte: u8) -> bool { 48 <= byte <= 57 || 97 <= byte <= 102 }
    pub open spec fn hex_suffix(bytes: Seq<u8>, start: int) -> bool {
        0 <= start <= bytes.len() && bytes.len() - start == 64
            && forall|i: int| start <= i < bytes.len() ==> hex_digit(bytes[i])
    }
    pub open spec fn handoff_id(value: Seq<char>) -> bool { hex_suffix(vstd::utf8::encode_utf8(value), 0) }
    pub open spec fn staged_key(value: Seq<char>) -> bool {
        let bytes = vstd::utf8::encode_utf8(value);
        crate::framing::range_matches(bytes, 0, seq![115u8, 116, 97, 103, 101, 100, 58])
            && hex_suffix(bytes, 7)
    }
    pub open spec fn receipt_policy(wire: Wire) -> bool {
        !wire.has_receipts || (wire.has_owners
            && (forall|i: int| 0 <= i < wire.receipts.len() ==> handoff_id(wire.receipts[i]@))
            && (forall|i: int| 0 <= i < wire.owners.len() ==> wire.owners[i].1.is_none()))
    }
    pub open spec fn child_names(children: Seq<String>) -> bool {
        forall|i: int| 0 <= i < children.len() ==> staged_key(children[i]@)
    }
    pub open spec fn edge_name(map: Map<String, Vec<String>>, key: String) -> bool {
        map.contains_key(key) && staged_key(key@) && child_names(map[key]@)
    }
    pub open spec fn graph_names(map: Map<String, Vec<String>>) -> bool {
        forall|key: String| map.contains_key(key) ==> edge_name(map, key)
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == hex_suffix(bytes@, start as int),
))]
// Keep primitive comparisons visible to the shared executable proof.
#[allow(clippy::manual_range_contains)]
fn valid_hex(bytes: &[u8], start: usize) -> bool {
    if start > bytes.len() || bytes.len() - start != 64 {
        return false;
    }
    let mut i = start;
    #[cfg_attr(straw_verus, verus_spec(
        invariant start <= i <= bytes.len(), bytes.len() - start == 64,
            forall|j: int| start <= j < i ==> hex_digit(bytes[j]),
        decreases bytes.len() - i,
    ))]
    while i < bytes.len() {
        let byte = bytes[i];
        if !((b'0' <= byte && byte <= b'9') || (b'a' <= byte && byte <= b'f')) {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == handoff_id(id@)))]
pub fn valid_handoff_id(id: &str) -> bool {
    valid_hex(id.as_bytes(), 0)
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == staged_key(key@)))]
pub fn valid_staged_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    let prefix = [115u8, 116, 97, 103, 101, 100, 58];
    crate::framing::matches_at(bytes, 0, &prefix) && valid_hex(bytes, 7)
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == receipt_policy(*wire)))]
pub fn valid_receipts(wire: &Wire) -> bool {
    if !wire.has_receipts {
        return true;
    }
    if !wire.has_owners {
        return false;
    }
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= wire.receipts.len(), wire.has_receipts, wire.has_owners,
            forall|j: int| 0 <= j < i ==> handoff_id(wire.receipts[j]@),
        decreases wire.receipts.len() - i,
    ))]
    while i < wire.receipts.len() {
        if !valid_handoff_id(&wire.receipts[i]) {
            return false;
        }
        i += 1;
    }
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= wire.owners.len(), wire.has_receipts, wire.has_owners,
            forall|j: int| 0 <= j < wire.receipts.len() ==> handoff_id(wire.receipts[j]@),
            forall|j: int| 0 <= j < i ==> wire.owners[j].1.is_none(),
        decreases wire.owners.len() - i,
    ))]
    while i < wire.owners.len() {
        if wire.owners[i].1.is_some() {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == child_names(children@)))]
fn valid_child_names(children: &[String]) -> bool {
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= children.len(), forall|j: int| 0 <= j < i ==> staged_key(children[j]@),
        decreases children.len() - i,
    ))]
    while i < children.len() {
        if !valid_staged_key(&children[i]) {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures result == (graph_names(additions@) && crate::consumption::compatible_graph(known@, additions@)),
))]
pub fn valid_staging_edges(
    known: &HashMap<String, Vec<String>>,
    additions: &HashMap<String, Vec<String>>,
) -> bool {
    let mut entries = additions.iter();
    #[cfg_attr(straw_verus, verus_spec(
        invariant entries.decrease().is_some(), crate::environment::string_hash_contract(),
            entries.remaining().unref().to_set().subset_of(additions@.kv_pairs()),
            forall|key: String| additions@.contains_key(key) ==>
                entries.remaining().unref().to_set().contains((key, additions@[key])) || edge_name(additions@, key),
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        #[cfg(straw_verus)]
        proof! {
            if entries.remaining().len() > 0 {
                crate::catalog_state::pop_pairs(entries.remaining());
                let key = *entries.remaining()[0].0;
                let children = *entries.remaining()[0].1;
                assert(entries.remaining().unref()[0] == (key, children));
                assert(entries.remaining().unref().to_set().contains((key, children)));
                assert(additions@.kv_pairs().contains((key, children)));
            }
        }
        let Some((key, children)) = entries.next() else {
            return crate::consumption::compatible(known, additions);
        };
        #[cfg(straw_verus)]
        proof! { assert(additions@.contains_key(*key) && additions@[*key] == *children); }
        if !valid_staged_key(key) || !valid_child_names(children) {
            #[cfg(straw_verus)]
            proof! { assert(!edge_name(additions@, *key)); }
            return false;
        }
        #[cfg(straw_verus)]
        proof! { assert(edge_name(additions@, *key)); }
    }
}
