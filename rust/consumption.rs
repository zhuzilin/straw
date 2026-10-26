//! A persisted staging graph must contain every referenced node. The caller
//! binds these keys/edges to authenticated immutable manifests and one frame.
use std::collections::{HashMap, HashSet};
#[cfg(straw_verus)]
use vstd::prelude::*;
#[cfg(straw_verus)]
use vstd::std_specs::iter::IteratorSpec;

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (left.deep_view() == right.deep_view()),
))]
pub fn same_children(left: &[String], right: &[String]) -> bool {
    #[cfg(straw_verus)]
    proof! {
        assert(left.deep_view().len() == left.len());
        assert(right.deep_view().len() == right.len());
    }
    if left.len() != right.len() {
        return false;
    }
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= left.len(), left.len() == right.len(),
            forall|j: int| 0 <= j < i ==> left[j]@ == right[j]@,
        decreases left.len() - i,
    ))]
    while i < left.len() {
        if !String::eq(&left[i], &right[i]) {
            #[cfg(straw_verus)]
            proof! {
                assert(left.deep_view()[i as int] == left[i as int]@);
                assert(right.deep_view()[i as int] == right[i as int]@);
            }
            return false;
        }
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! { assert(left.deep_view() =~= right.deep_view()); }
    true
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn compatible_entry(known: Map<String, Vec<String>>, additions: Map<String, Vec<String>>, key: String) -> bool {
        additions.contains_key(key)
        && (!known.contains_key(key) || known[key].deep_view() == additions[key].deep_view())
        && forall|i: int| 0 <= i < additions[key]@.len() ==>
            known.contains_key(additions[key]@[i]) || additions.contains_key(additions[key]@[i])
    }
    pub open spec fn compatible_graph(known: Map<String, Vec<String>>, additions: Map<String, Vec<String>>) -> bool {
        forall|key: String| additions.contains_key(key) ==> compatible_entry(known, additions, key)
    }
}

/// Check the whole update graph before applying any of its edges. Every child
/// must have a definition, and stored immutable edges cannot be changed.
#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures result == compatible_graph(known@, additions@),
))]
pub fn compatible(
    known: &HashMap<String, Vec<String>>,
    additions: &HashMap<String, Vec<String>>,
) -> bool {
    let mut entries = additions.iter();
    #[cfg_attr(straw_verus, verus_spec(
        invariant entries.decrease().is_some(), crate::environment::string_hash_contract(),
            entries.remaining().unref().to_set().subset_of(additions@.kv_pairs()),
            forall|key: String| additions@.contains_key(key) ==>
                entries.remaining().unref().to_set().contains((key, additions@[key]))
                || compatible_entry(known@, additions@, key),
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
                assert(additions@.contains_key(key) && additions@[key] == children);
            }
        }
        let Some((key, children)) = entries.next() else {
            return true;
        };
        #[cfg(straw_verus)]
        proof! { assert(additions@.contains_key(*key) && additions@[*key] == *children); }
        if !covered(known, additions, children) {
            #[cfg(straw_verus)]
            proof! { assert(!compatible_entry(known@, additions@, *key)); }
            return false;
        }
        let consistent = match known.get(key) {
            Some(previous) => same_children(previous, children),
            None => true,
        };
        if !consistent {
            #[cfg(straw_verus)]
            proof! { assert(!compatible_entry(known@, additions@, *key)); }
            return false;
        }
        #[cfg(straw_verus)]
        proof! { assert(compatible_entry(known@, additions@, *key)); }
    }
}

// An error may follow the durable receipt. Keep the exact outstanding work
// until the catalog confirms it; a later invocation can recognize that receipt.
#[cfg_attr(straw_verus, verus_spec(
    requires old(pending).len() > 0,
    ensures final(pending)@ == (if confirmed { old(pending)@.drop_last() } else { old(pending)@ }),
))]
pub fn pop_completed<T>(pending: &mut Vec<T>, confirmed: bool) {
    if confirmed {
        pending.pop();
    }
}

#[cfg_attr(straw_verus, verus_spec(
    requires crate::environment::string_hash_contract(),
    ensures final(known)@ == old(known)@.union(receipts@.to_set()),
))]
pub fn record_receipts(known: &mut HashSet<String>, receipts: &[String]) {
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= receipts.len(), crate::environment::string_hash_contract(),
            forall|key: String| known@.contains(key) ==
                (old(known)@.contains(key) || exists|j: int| 0 <= j < i && receipts[j] == key),
        decreases receipts.len() - i,
    ))]
    while i < receipts.len() {
        known.insert(receipts[i].clone());
        i += 1;
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures result == (forall|i: int| 0 <= i < children.len() ==>
        known@.dom().contains(children[i]) || additions@.dom().contains(children[i])),
))]
pub fn covered(
    known: &HashMap<String, Vec<String>>,
    additions: &HashMap<String, Vec<String>>,
    children: &[String],
) -> bool {
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= children.len(), crate::environment::string_hash_contract(),
            forall|j: int| 0 <= j < i ==>
                known@.dom().contains(children[j]) || additions@.dom().contains(children[j]),
        decreases children.len() - i,
    ))]
    while i < children.len() {
        if !known.contains_key(&children[i]) && !additions.contains_key(&children[i]) {
            return false;
        }
        i += 1;
    }
    true
}
