//! A replayed prefix is tentative until its durability barrier succeeds.

pub struct Recovery {
    pub durable: u64,
    pub rebuild: bool,
}

pub fn start() -> Recovery {
    Recovery {
        durable: 0,
        rebuild: false,
    }
}

pub fn ready(state: &Recovery) -> bool {
    !state.rebuild
}

pub fn buffer_capacity(remaining: u64) -> u64 {
    // Bound read-ahead independently of frame lengths and total log history.
    if remaining < 65536 { remaining } else { 65536 }
}

pub fn prepare(state: &mut Recovery, changes: bool) {
    // Mark before any tentative mutation, including a partially failed apply.
    // A failed empty refresh leaves a previously confirmed cache reusable.
    state.rebuild = state.rebuild || changes;
}

pub fn accepts(state: &Recovery, position: u64) -> bool {
    position >= state.durable
}

pub fn confirm(state: &mut Recovery, position: u64) -> bool {
    if position < state.durable {
        return false;
    }
    state.durable = position;
    state.rebuild = false;
    true
}
