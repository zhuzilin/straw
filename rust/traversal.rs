//! Heap-backed frontier. Scheduling deduplicates before expansion, including
//! repeated edges and cycles; the caller supplies immutable node identities.
use std::collections::HashSet;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub struct Frontier<T> {
    pub seen: HashSet<String>,
    pub pending: Vec<(String, T)>,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures result.seen@ == Set::<String>::empty(), result.pending@ == Seq::<(String, T)>::empty(), valid(&result),
))]
pub fn start<T>() -> Frontier<T> {
    Frontier {
        seen: HashSet::new(),
        pending: Vec::new(),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(), valid(old(state)),
    ensures result == !old(state).seen@.contains(key),
        final(state).seen@ == old(state).seen@.insert(key),
        final(state).pending@ == if result { old(state).pending@.push((key, value)) } else { old(state).pending@ },
        valid(final(state)),
))]
pub fn schedule<T>(state: &mut Frontier<T>, key: String, value: T) -> bool {
    if !state.seen.insert(key.clone()) {
        return false;
    }
    state.pending.push((key, value));
    true
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires valid(old(state)),
    ensures final(state).seen@ == old(state).seen@, valid(final(state)),
        result == if old(state).pending@.len() == 0 { None } else { Some(old(state).pending@.last()) },
        final(state).pending@ == if old(state).pending@.len() == 0 { old(state).pending@ } else { old(state).pending@.drop_last() },
))]
pub fn next<T>(state: &mut Frontier<T>) -> Option<(String, T)> {
    state.pending.pop()
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn valid<T>(state: &Frontier<T>) -> bool {
        (forall|i: int| 0 <= i < state.pending@.len() ==> state.seen@.contains(state.pending@[i].0))
        && (forall|i: int, j: int| 0 <= i < j < state.pending@.len() ==> state.pending@[i].0 != state.pending@[j].0)
    }
}
