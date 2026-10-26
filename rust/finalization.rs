//! Every publication stages its roots; standalone files must then be sealed.
//! I/O errors may have effects, so a new invocation repeats both idempotent steps.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Api {
    Records,
    RecordSet,
    Publish,
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == (left == right)))]
pub fn same_api(left: Api, right: Api) -> bool {
    match left {
        Api::Records => matches!(right, Api::Records),
        Api::RecordSet => matches!(right, Api::RecordSet),
        Api::Publish => matches!(right, Api::Publish),
    }
}

// The exact completed-file description remains bound to its API/request until
// staging, sealing and any internal ownership handoff have all returned success.
#[cfg_attr(straw_verus, verus_verify)]
pub struct Handoff<T> {
    pub value: T,
    pub api: Api,
    pub active: bool,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.value == value, result.api == api, result.active,
))]
pub fn handoff<T>(value: T, api: Api) -> Handoff<T> {
    Handoff {
        value,
        api,
        active: true,
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (!state.active || (state.api == api && same_id)),
))]
pub fn allows<T>(state: &Handoff<T>, api: Api, same_id: bool) -> bool {
    !state.active || (same_api(state.api, api) && same_id)
}

#[cfg_attr(straw_verus, verus_spec(
    ensures final(state).value == old(state).value, final(state).api == old(state).api,
        !final(state).active,
))]
pub fn acknowledge<T>(state: &mut Handoff<T>) {
    state.active = false;
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Stage,
    Seal,
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug)]
pub struct State {
    pub completed: u8,
    pub failed: bool,
    pub standalone: bool,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result@ == crate::protocol::initial(), result.standalone == standalone,
))]
pub fn start(standalone: bool) -> State {
    State {
        completed: 0,
        failed: false,
        standalone,
    }
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == effect(state@, state.standalone)))]
pub fn next(state: &State) -> Option<Effect> {
    if state.failed {
        return None;
    }
    match state.completed {
        0 => Some(Effect::Stage),
        1 if state.standalone => Some(Effect::Seal),
        _ => None,
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.standalone == state.standalone,
        result@ == crate::protocol::advance(state@, success, limit(state.standalone)),
        state@.completed <= limit(state.standalone) ==> result@.completed <= limit(state.standalone),
))]
pub fn advance(state: State, success: bool) -> State {
    let limit = if state.standalone { 2 } else { 1 };
    if state.failed || state.completed >= limit {
        return state;
    }
    State {
        completed: state.completed + if success { 1 } else { 0 },
        failed: !success,
        standalone: state.standalone,
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (!state@.failed && state@.completed == limit(state.standalone)),
))]
pub fn finished(state: &State) -> bool {
    !state.failed && state.completed == if state.standalone { 2 } else { 1 }
}

#[cfg(straw_verus)]
verus! {
    impl View for State {
        type V = crate::protocol::ProgressView;
        open spec fn view(&self) -> crate::protocol::ProgressView {
            crate::protocol::ProgressView { completed: self.completed as nat, failed: self.failed }
        }
    }
    pub open spec fn limit(standalone: bool) -> nat { if standalone { 2 } else { 1 } }
    pub open spec fn effect(state: crate::protocol::ProgressView, standalone: bool) -> Option<Effect> {
        if state.failed { None }
        else if state.completed == 0 { Some(Effect::Stage) }
        else if standalone && state.completed == 1 { Some(Effect::Seal) }
        else { None }
    }
}
