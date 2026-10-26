//! Every publication stages its roots; standalone files must then be sealed.
//! I/O errors may have effects, so a new invocation repeats both idempotent steps.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Api {
    Records,
    RecordSet,
    Publish,
}

pub fn same_api(left: Api, right: Api) -> bool {
    match left {
        Api::Records => matches!(right, Api::Records),
        Api::RecordSet => matches!(right, Api::RecordSet),
        Api::Publish => matches!(right, Api::Publish),
    }
}

// The exact completed-file description remains bound to its API/request until
// staging, sealing and any internal ownership handoff have all returned success.

pub struct Handoff<T> {
    pub value: T,
    pub api: Api,
    pub active: bool,
}

pub fn handoff<T>(value: T, api: Api) -> Handoff<T> {
    Handoff {
        value,
        api,
        active: true,
    }
}

pub fn allows<T>(state: &Handoff<T>, api: Api, same_id: bool) -> bool {
    !state.active || (same_api(state.api, api) && same_id)
}

pub fn acknowledge<T>(state: &mut Handoff<T>) {
    state.active = false;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Stage,
    Seal,
}

#[derive(Clone, Copy, Debug)]
pub struct State {
    pub completed: u8,
    pub failed: bool,
    pub standalone: bool,
}

pub fn start(standalone: bool) -> State {
    State {
        completed: 0,
        failed: false,
        standalone,
    }
}

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

pub fn finished(state: &State) -> bool {
    !state.failed && state.completed == if state.standalone { 2 } else { 1 }
}
