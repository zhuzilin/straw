//! Preserve the first occurrence of each scheduling key before issuing leases.
use std::collections::BTreeSet;
#[cfg(straw_verus)]
use vstd::prelude::*;

pub type Key = (i64, i64, usize);

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures
        result.len() == keys.len(),
        forall|i: int| 0 <= i < keys.len() ==>
            result[i] == (forall|j: int| 0 <= j < i ==> keys[j] != keys[i]),
))]
pub fn first_occurrences(keys: &[Key]) -> Vec<bool> {
    let mut seen: BTreeSet<Key> = BTreeSet::new();
    let mut result: Vec<bool> = Vec::new();
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant
            i <= keys.len(), result.len() == i,
            forall|key: Key| seen@.contains(key) == (exists|j: int| 0 <= j < i && keys[j] == key),
            forall|k: int| 0 <= k < i ==>
                result[k] == (forall|j: int| 0 <= j < k ==> keys[j] != keys[k]),
        decreases keys.len() - i,
    ))]
    while i < keys.len() {
        result.push(seen.insert(keys[i]));
        i += 1;
    }
    result
}
