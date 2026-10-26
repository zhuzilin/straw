//! Exact-length guard for caller-supplied payload traversals.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub struct Length {
    pub expected: u64,
    pub seen: u64,
    pub failed: bool,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.expected == expected, result.seen == 0, !result.failed,
))]
pub fn start(expected: u64) -> Length {
    Length {
        expected,
        seen: 0,
        failed: false,
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures
        final(state).expected == old(state).expected,
        result == (!old(state).failed && old(state).seen as int + count as int <= old(state).expected as int),
        final(state).failed == (old(state).failed || !result),
        final(state).seen as int == if result { old(state).seen as int + count as int } else { old(state).seen as int },
        old(state).seen <= old(state).expected ==> final(state).seen <= final(state).expected,
))]
pub fn accept(state: &mut Length, count: u64) -> bool {
    if state.failed || state.seen > state.expected || count > state.expected - state.seen {
        state.failed = true;
        return false;
    }
    state.seen += count;
    true
}

#[cfg_attr(straw_verus, verus_spec(
    ensures final(state).failed, final(state).expected == old(state).expected,
        final(state).seen == old(state).seen,
))]
pub fn fail(state: &mut Length) {
    state.failed = true;
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (!state.failed && state.seen == state.expected),
))]
pub fn complete(state: &Length) -> bool {
    !state.failed && state.seen == state.expected
}
