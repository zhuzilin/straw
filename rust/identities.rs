//! Memoized namespace identities under a stable, correctly resolved namespace.
//! Resolution itself is an I/O adapter obligation, not proved by this cache.
use std::collections::HashMap;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (expected_root@ == actual_root@ && expected_run@ == actual_run@),
))]
pub fn same_pool(
    expected_root: &str,
    expected_run: &str,
    actual_root: &str,
    actual_run: &str,
) -> bool {
    expected_root == actual_root && expected_run == actual_run
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (expected == observed),
))]
pub fn same_log(expected: (u64, u64), observed: (u64, u64)) -> bool {
    expected.0 == observed.0 && expected.1 == observed.1
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Legacy,
    Resolved,
    Unsupported,
}

/// A missing marker is legacy. Only version 1 certifies normalized path keys.
/// JSON type validation and the writer's normalization are adapter obligations.
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures
        (result == Encoding::Legacy) == version.is_none(),
        (result == Encoding::Resolved) == (version == Some(1u64)),
        (result == Encoding::Unsupported) == (version.is_some() && version != Some(1u64)),
))]
pub fn encoding(version: Option<u64>) -> Encoding {
    match version {
        None => Encoding::Legacy,
        Some(1) => Encoding::Resolved,
        Some(_) => Encoding::Unsupported,
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures result == if cache@.dom().contains(*path) { Some(cache@[*path]) } else { None },
))]
pub fn lookup(cache: &HashMap<String, String>, path: &String) -> Option<String> {
    cache.get(path).cloned()
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures
        result == (!old(cache)@.dom().contains(path) || old(cache)@[path]@ == identity@),
        final(cache)@ == if old(cache)@.dom().contains(path) { old(cache)@ } else { old(cache)@.insert(path, identity) },
))]
pub fn remember(cache: &mut HashMap<String, String>, path: String, identity: String) -> bool {
    if let Some(previous) = cache.get(&path) {
        return String::eq(previous, &identity);
    }
    cache.insert(path, identity);
    true
}
