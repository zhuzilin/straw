//! A replayed prefix is tentative until its durability barrier succeeds.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub struct Recovery {
    pub durable: u64,
    pub rebuild: bool,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.durable == 0, !result.rebuild,
))]
pub fn start() -> Recovery {
    Recovery {
        durable: 0,
        rebuild: false,
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == !state.rebuild,
))]
pub fn ready(state: &Recovery) -> bool {
    !state.rebuild
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == if remaining < 65536 { remaining } else { 65536 },
        result <= remaining, result <= 65536,
        (result == 0) == (remaining == 0),
))]
pub fn buffer_capacity(remaining: u64) -> u64 {
    // Bound read-ahead independently of frame lengths and total log history.
    if remaining < 65536 { remaining } else { 65536 }
}

#[cfg_attr(straw_verus, verus_spec(
    ensures final(state).durable == old(state).durable,
        final(state).rebuild == (old(state).rebuild || changes),
))]
pub fn prepare(state: &mut Recovery, changes: bool) {
    // Mark before any tentative mutation, including a partially failed apply.
    // A failed empty refresh leaves a previously confirmed cache reusable.
    state.rebuild = state.rebuild || changes;
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (position >= state.durable),
))]
pub fn accepts(state: &Recovery, position: u64) -> bool {
    position >= state.durable
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (position >= old(state).durable),
        final(state).durable == if result { position } else { old(state).durable },
        final(state).rebuild == if result { false } else { old(state).rebuild },
        final(state).durable >= old(state).durable,
))]
pub fn confirm(state: &mut Recovery, position: u64) -> bool {
    if position < state.durable {
        return false;
    }
    state.durable = position;
    state.rebuild = false;
    true
}
