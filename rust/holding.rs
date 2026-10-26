//! An in-flight publication may add protection, but cannot drop earlier roots
//! before its successful staging handoff. Ordinary owners still use replace.
use std::collections::BTreeSet;
#[cfg(straw_verus)]
use vstd::prelude::*;

// Keep an uncertain preparation owner until its release succeeds or the
// complete publication takes over responsibility for protecting it.
#[cfg_attr(straw_verus, verus_spec(
    ensures *final(owner) == (if safe { None } else { *old(owner) }),
))]
pub fn finish<T>(owner: &mut Option<T>, safe: bool) {
    if safe {
        *owner = None;
    }
}

#[cfg_attr(straw_verus, verus_spec(
    requires crate::environment::string_order_contract(),
    ensures final(kept)@ == old(kept)@.union(additions@.to_set()),
))]
pub fn extend(kept: &mut BTreeSet<String>, additions: &[String]) {
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= additions.len(), crate::environment::string_order_contract(),
            forall|key: String| kept@.contains(key) ==
                (old(kept)@.contains(key) || exists|j: int| 0 <= j < i && additions[j] == key),
        decreases additions.len() - i,
    ))]
    while i < additions.len() {
        kept.insert(additions[i].clone());
        i += 1;
    }
}
