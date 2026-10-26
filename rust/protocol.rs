//! Runtime deletion ordering and ownership decisions.
//! Queue commits execute through queue_commit.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteEffect {
    Unlink,
    SyncParent,
}

#[derive(Clone, Copy, Debug)]
pub struct DeleteProtocol {
    pub completed: u8,
    pub failed: bool,
    pub tombstoned: bool,
}

pub fn delete_start(tombstoned: bool) -> DeleteProtocol {
    DeleteProtocol {
        completed: 0,
        failed: false,
        tombstoned,
    }
}

pub fn delete_next(state: &DeleteProtocol) -> Option<DeleteEffect> {
    if state.failed || !state.tombstoned {
        return None;
    }
    match state.completed {
        0 => Some(DeleteEffect::Unlink),
        1 => Some(DeleteEffect::SyncParent),
        _ => None,
    }
}

pub fn delete_advance(state: DeleteProtocol, success: bool) -> DeleteProtocol {
    if state.failed || state.completed >= 2 || !state.tombstoned {
        return state;
    }
    if success {
        DeleteProtocol {
            completed: state.completed + 1,
            failed: false,
            tombstoned: true,
        }
    } else {
        DeleteProtocol {
            completed: state.completed,
            failed: true,
            tombstoned: true,
        }
    }
}

pub fn delete_finished(state: &DeleteProtocol) -> bool {
    state.tombstoned && !state.failed && state.completed == 2
}

/// Complete a batch only after every parent barrier succeeded. Each member
/// must be waiting for that barrier; an already-finished or failed member is
/// rejected as well. Directory/path coverage belongs to the I/O adapter.
pub fn delete_batch_finished(states: &[DeleteProtocol], synced: bool) -> bool {
    if !synced {
        return false;
    }
    let mut i = 0usize;

    while i < states.len() {
        if !matches!(delete_next(&states[i]), Some(DeleteEffect::SyncParent)) {
            return false;
        }
        let completed = delete_advance(states[i], true);
        if !delete_finished(&completed) {
            return false;
        }
        i += 1;
    }
    true
}

#[derive(Clone, Copy, Debug)]
pub enum ReclaimEffect {
    SyncParents,
    RecordReceipt,
}

#[derive(Clone, Copy, Debug)]
pub struct ReclaimProtocol {
    pub completed: u8,
    pub failed: bool,
}

pub fn reclaim_start() -> ReclaimProtocol {
    ReclaimProtocol {
        completed: 0,
        failed: false,
    }
}

pub fn reclaim_next(state: &ReclaimProtocol) -> Option<ReclaimEffect> {
    if state.failed {
        return None;
    }
    match state.completed {
        0 => Some(ReclaimEffect::SyncParents),
        1 => Some(ReclaimEffect::RecordReceipt),
        _ => None,
    }
}

pub fn reclaim_advance(state: ReclaimProtocol, success: bool) -> ReclaimProtocol {
    if state.failed || state.completed >= 2 {
        return state;
    }
    if success {
        ReclaimProtocol {
            completed: state.completed + 1,
            failed: false,
        }
    } else {
        ReclaimProtocol {
            completed: state.completed,
            failed: true,
        }
    }
}

pub fn reclaim_finished(state: &ReclaimProtocol) -> bool {
    !state.failed && state.completed == 2
}

#[derive(Clone, Copy, Debug)]
pub struct PackStatus {
    pub sealed: bool,
    pub owned: bool,
    pub tombstoned: bool,
}

/// Final authorization uses a fresh locked snapshot, not the pre-scan one.
pub fn reactivation_permitted(verified: bool, statuses: &[PackStatus]) -> bool {
    if !verified || statuses.is_empty() {
        return false;
    }
    let mut i = 0usize;

    while i < statuses.len() {
        if !statuses[i].owned || statuses[i].tombstoned {
            return false;
        }
        i += 1;
    }
    true
}

pub fn candidate(status: PackStatus) -> bool {
    status.sealed && !status.owned && !status.tombstoned
}
