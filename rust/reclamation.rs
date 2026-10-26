//! Replayable deletion index. Tombstones are permanent; retry work is not.
use std::collections::HashSet;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_spec(
    requires crate::environment::string_hash_contract(),
        old(pending)@.subset_of(old(marked)@),
    ensures
        final(marked)@ == old(marked)@.insert(path),
        final(pending)@ == if old(marked)@.contains(path) { old(pending)@ } else { old(pending)@.insert(path) },
        final(pending)@.subset_of(final(marked)@),
))]
pub fn mark(marked: &mut HashSet<String>, pending: &mut HashSet<String>, path: String) {
    // Replaying a tombstone after its receipt must not schedule it again.
    if marked.insert(path.clone()) {
        pending.insert(path);
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(), old(pending)@.subset_of(marked@),
    ensures result == marked@.contains(*path),
        final(pending)@ == if result { old(pending)@.remove(*path) } else { old(pending)@ },
        final(pending)@.subset_of(marked@),
))]
pub fn retire(marked: &HashSet<String>, pending: &mut HashSet<String>, path: &String) -> bool {
    if !marked.contains(path) {
        return false;
    }
    pending.remove(path);
    true
}
