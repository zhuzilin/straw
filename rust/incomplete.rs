//! Roll back an unpublished write before reusing/sealing its writer.
//! The descriptor (including the exact saved prefix) survives failed cleanup.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_spec(result => ensures result == (owner == current)))]
pub fn writer_process(owner: u32, current: u32) -> bool {
    owner == current
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Truncate,
    SyncFile,
    UnlinkPartial,
    SyncParent,
    SealFinal,
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct State<T> {
    pub value: T,
    pub completed: u8,
    pub failed: bool,
    pub standalone: bool,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.value == value, result.standalone == standalone,
        result@ == crate::protocol::initial(),
))]
pub fn start<T>(value: T, standalone: bool) -> State<T> {
    State {
        value,
        completed: 0,
        failed: false,
        standalone,
    }
}

// A newly allocated private name may be registered or created even when setup
// reports an error. Until offset capture succeeds, retire the whole new name.
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.value.0 == value, result.value.1 == 0,
        result.standalone, result@ == crate::protocol::initial(),
))]
pub fn preparing<T>(value: T) -> State<(T, u64)> {
    start((value, 0), true)
}

#[cfg_attr(straw_verus, verus_spec(
    requires old(state).standalone, old(state).value.1 == 0,
        old(state)@ == crate::protocol::initial(),
    ensures final(state).value.0 == old(state).value.0,
        final(state).value.1 == prefix, final(state).standalone == standalone,
        final(state)@ == old(state)@,
))]
pub fn capture_prefix<T>(state: &mut State<(T, u64)>, prefix: u64, standalone: bool) {
    state.value.1 = prefix;
    state.standalone = standalone;
}

#[cfg_attr(straw_verus, verus_spec(
    ensures final(state).value == old(state).value,
        final(state).standalone == old(state).standalone,
        final(state)@ == crate::protocol::initial(),
))]
pub fn restart<T>(state: &mut State<T>) {
    state.completed = 0;
    state.failed = false;
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == effect(state@, state.standalone)))]
pub fn next<T>(state: &State<T>) -> Option<Effect> {
    if state.failed {
        return None;
    }
    match state.completed {
        0 if state.standalone => Some(Effect::UnlinkPartial),
        0 => Some(Effect::Truncate),
        1 if state.standalone => Some(Effect::SyncParent),
        1 => Some(Effect::SyncFile),
        2 if state.standalone => Some(Effect::SealFinal),
        _ => None,
    }
}

#[cfg_attr(straw_verus, verus_spec(
    ensures final(state).value == old(state).value,
        final(state).standalone == old(state).standalone,
        final(state)@ == crate::protocol::advance(old(state)@, success, limit(old(state).standalone)),
        old(state)@.completed <= limit(old(state).standalone) ==> final(state)@.completed <= limit(final(state).standalone),
))]
pub fn advance<T>(state: &mut State<T>, success: bool) {
    let limit = if state.standalone { 3 } else { 2 };
    if !state.failed && state.completed < limit {
        state.completed += if success { 1 } else { 0 };
        state.failed = !success;
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (!state@.failed && state@.completed == limit(state.standalone)),
))]
pub fn finished<T>(state: &State<T>) -> bool {
    !state.failed && state.completed == if state.standalone { 3 } else { 2 }
}

#[cfg(straw_verus)]
verus! {
    impl<T> View for State<T> {
        type V = crate::protocol::ProgressView;
        open spec fn view(&self) -> crate::protocol::ProgressView {
            crate::protocol::ProgressView { completed: self.completed as nat, failed: self.failed }
        }
    }
    pub open spec fn limit(standalone: bool) -> nat { if standalone { 3 } else { 2 } }
    pub open spec fn effect(state: crate::protocol::ProgressView, standalone: bool) -> Option<Effect> {
        if state.failed { None }
        else if state.completed == 0 { Some(if standalone { Effect::UnlinkPartial } else { Effect::Truncate }) }
        else if state.completed == 1 { Some(if standalone { Effect::SyncParent } else { Effect::SyncFile }) }
        else if state.completed == 2 && standalone { Some(Effect::SealFinal) }
        else { None }
    }
}
