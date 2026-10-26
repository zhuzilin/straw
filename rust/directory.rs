//! A directory is acknowledged only after every ancestor barrier succeeds.
//! Path enumeration and filesystem effects are adapter obligations.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParentAction {
    Sync,
    RecreateAndSync,
    Fail,
}

// Missing is an error classification, not proof of durable absence. A repair
// must establish the directory and complete its ancestor barriers.
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == if opened { ParentAction::Sync }
        else if missing { ParentAction::RecreateAndSync } else { ParentAction::Fail },
))]
pub fn parent_action(opened: bool, missing: bool) -> ParentAction {
    if opened {
        ParentAction::Sync
    } else if missing {
        ParentAction::RecreateAndSync
    } else {
        ParentAction::Fail
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Barriers {
    pub total: u64,
    pub completed: u64,
    pub failed: bool,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.total == total, result.completed == 0, !result.failed,
))]
pub fn start(total: u64) -> Barriers {
    Barriers {
        total,
        completed: 0,
        failed: false,
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == if !state.failed && state.completed < state.total {
        Some(state.completed)
    } else { None },
))]
pub fn next(state: &Barriers) -> Option<u64> {
    if !state.failed && state.completed < state.total {
        Some(state.completed)
    } else {
        None
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.total == state.total,
        result.completed as int == if !state.failed && state.completed < state.total && success {
            state.completed as int + 1
        } else { state.completed as int },
        result.failed == (state.failed || (state.completed < state.total && !success)),
))]
pub fn advance(state: Barriers, success: bool) -> Barriers {
    if state.failed || state.completed >= state.total {
        return state;
    }
    Barriers {
        total: state.total,
        completed: if success {
            state.completed + 1
        } else {
            state.completed
        },
        failed: !success,
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (!state.failed && state.completed == state.total),
))]
pub fn finished(state: &Barriers) -> bool {
    !state.failed && state.completed == state.total
}
