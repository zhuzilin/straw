//! Replayable deletion index. Tombstones are permanent; retry work is not.
use std::collections::HashSet;

pub fn mark(marked: &mut HashSet<String>, pending: &mut HashSet<String>, path: String) {
    // Replaying a tombstone after its receipt must not schedule it again.
    if marked.insert(path.clone()) {
        pending.insert(path);
    }
}

pub fn retire(marked: &HashSet<String>, pending: &mut HashSet<String>, path: &String) -> bool {
    if !marked.contains(path) {
        return false;
    }
    pending.remove(path);
    true
}
