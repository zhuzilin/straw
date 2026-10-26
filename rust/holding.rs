//! An in-flight publication may add protection, but cannot drop earlier roots
//! before its successful staging handoff. Ordinary owners still use replace.
use std::collections::BTreeSet;

// Keep an uncertain preparation owner until its release succeeds or the
// complete publication takes over responsibility for protecting it.

pub fn finish<T>(owner: &mut Option<T>, safe: bool) {
    if safe {
        *owner = None;
    }
}

pub fn extend(kept: &mut BTreeSet<String>, additions: &[String]) {
    let mut i = 0usize;

    while i < additions.len() {
        kept.insert(additions[i].clone());
        i += 1;
    }
}
