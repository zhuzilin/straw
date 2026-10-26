//! Memoized namespace identities under a stable, correctly resolved namespace.
//! The caller resolves paths; this cache binds their canonical identities.
use std::collections::HashMap;

pub fn same_pool(
    expected_root: &str,
    expected_run: &str,
    actual_root: &str,
    actual_run: &str,
) -> bool {
    expected_root == actual_root && expected_run == actual_run
}

pub fn same_log(expected: (u64, u64), observed: (u64, u64)) -> bool {
    expected.0 == observed.0 && expected.1 == observed.1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Legacy,
    Resolved,
    Unsupported,
}

/// A missing marker is legacy. Only version 1 certifies normalized path keys.
/// JSON type validation and the writer's normalization are adapter obligations.
pub fn encoding(version: Option<u64>) -> Encoding {
    match version {
        None => Encoding::Legacy,
        Some(1) => Encoding::Resolved,
        Some(_) => Encoding::Unsupported,
    }
}

pub fn lookup(cache: &HashMap<String, String>, path: &String) -> Option<String> {
    cache.get(path).cloned()
}

pub fn remember(cache: &mut HashMap<String, String>, path: String, identity: String) -> bool {
    if let Some(previous) = cache.get(&path) {
        return String::eq(previous, &identity);
    }
    cache.insert(path, identity);
    true
}
