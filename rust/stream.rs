//! Exact-length guard for caller-supplied payload traversals.

pub struct Length {
    pub expected: u64,
    pub seen: u64,
    pub failed: bool,
}

pub fn start(expected: u64) -> Length {
    Length {
        expected,
        seen: 0,
        failed: false,
    }
}

pub fn accept(state: &mut Length, count: u64) -> bool {
    if state.failed || state.seen > state.expected || count > state.expected - state.seen {
        state.failed = true;
        return false;
    }
    state.seen += count;
    true
}

pub fn fail(state: &mut Length) {
    state.failed = true;
}

pub fn complete(state: &Length) -> bool {
    !state.failed && state.seen == state.expected
}
