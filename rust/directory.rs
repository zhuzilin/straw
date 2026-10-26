//! A directory is acknowledged only after every ancestor barrier succeeds.
//! Path enumeration and filesystem effects are adapter obligations.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParentAction {
    Sync,
    RecreateAndSync,
    Fail,
}

// Missing is an error classification, not proof of durable absence. A repair
// must establish the directory and complete its ancestor barriers.

pub fn parent_action(opened: bool, missing: bool) -> ParentAction {
    if opened {
        ParentAction::Sync
    } else if missing {
        ParentAction::RecreateAndSync
    } else {
        ParentAction::Fail
    }
}

pub struct Barriers {
    pub total: u64,
    pub completed: u64,
    pub failed: bool,
}

pub fn start(total: u64) -> Barriers {
    Barriers {
        total,
        completed: 0,
        failed: false,
    }
}

pub fn next(state: &Barriers) -> Option<u64> {
    if !state.failed && state.completed < state.total {
        Some(state.completed)
    } else {
        None
    }
}

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

pub fn finished(state: &Barriers) -> bool {
    !state.failed && state.completed == state.total
}
