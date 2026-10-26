//! Keep the exact completed-file description across an uncertain sync response.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub struct Pending<T> {
    pub value: T,
    pub synced: bool,
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result.value == value, !result.synced))]
pub fn start<T>(value: T) -> Pending<T> {
    Pending {
        value,
        synced: false,
    }
}

#[cfg_attr(straw_verus, verus_spec(
    ensures final(state).value == old(state).value,
        final(state).synced == (old(state).synced || success),
))]
pub fn record_sync<T>(state: &mut Pending<T>, success: bool) {
    state.synced = state.synced || success;
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == !state.synced))]
pub fn needs_sync<T>(state: &Pending<T>) -> bool {
    !state.synced
}
