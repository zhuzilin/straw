//! Native deletion controllers and verification-only commit ordering model.
//! Queue commits execute through queue_commit; the older abstract dispatcher is
//! retained only for its independent protocol proofs and mutation controls.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[cfg(straw_verus)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitEffect {
    Retain,
    AppendWal,
    Apply,
    ConsumeStaging,
}
#[cfg_attr(straw_verus, verus_verify)]
#[cfg(straw_verus)]
#[derive(Clone, Copy, Debug)]
pub struct CommitProtocol {
    pub completed: u8,
    pub failed: bool,
}
#[cfg_attr(straw_verus, verus_spec(result => ensures result@ == initial()))]
#[cfg(straw_verus)]
pub fn commit_start() -> CommitProtocol {
    CommitProtocol {
        completed: 0,
        failed: false,
    }
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == commit_effect(state@)))]
#[cfg(straw_verus)]
pub fn commit_next(state: &CommitProtocol) -> Option<CommitEffect> {
    if state.failed {
        return None;
    }
    match state.completed {
        0 => Some(CommitEffect::Retain),
        1 => Some(CommitEffect::AppendWal),
        2 => Some(CommitEffect::Apply),
        3 => Some(CommitEffect::ConsumeStaging),
        _ => None,
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result@ == advance(state@, success, 4),
        state@.completed <= 4 ==> result@.completed <= 4,
))]
#[cfg(straw_verus)]
pub fn commit_advance(state: CommitProtocol, success: bool) -> CommitProtocol {
    if state.failed || state.completed >= 4 {
        return state;
    }
    if success {
        CommitProtocol {
            completed: state.completed + 1,
            failed: false,
        }
    } else {
        CommitProtocol {
            completed: state.completed,
            failed: true,
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == (!state@.failed && state@.completed == 4)))]
#[cfg(straw_verus)]
pub fn commit_finished(state: &CommitProtocol) -> bool {
    !state.failed && state.completed == 4
}

#[cfg(straw_verus)]
verus! {
    /// State of this one immutable transaction, not the complete queue/catalog.
    pub struct CommitView {
        pub request: Seq<u8>,
        pub retained: bool,
        pub accepted: bool,
        pub applied: bool,
        pub consumed: bool,
        pub poisoned: bool,
    }
    pub trait CommitModel: View<V = CommitView> {}
    pub open spec fn commit_safe(s: CommitView) -> bool {
        (s.accepted ==> s.retained) && (s.applied ==> s.accepted) && (s.consumed ==> s.applied)
    }
    pub open spec fn commit_ready(s: CommitView) -> bool {
        !s.accepted && !s.applied && !s.consumed && !s.poisoned
    }
    pub open spec fn commit_stage(s: CommitView, completed: nat) -> bool {
        commit_safe(s) && !s.poisoned
            && (completed >= 1 ==> s.retained)
            && (s.accepted == (completed >= 2))
            && (s.applied == (completed >= 3))
            && (s.consumed == (completed >= 4))
    }
}

/// Lower effects preserve one transaction's identity. Their native mapping is
/// a separate obligation; this interface specifies individual lower effects.
#[cfg_attr(straw_verus, verus_verify)]
#[cfg(straw_verus)]
pub trait CommitEffects: CommitModel {
    type Error;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires commit_ready(old(self)@),
        ensures final(self)@ == (CommitView { retained: final(self)@.retained, ..old(self)@ }),
            result.is_ok() ==> final(self)@.retained,
    ))]
    fn retain(&mut self) -> Result<(), Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires commit_stage(old(self)@, 1),
        ensures final(self)@ == (CommitView { accepted: final(self)@.accepted,
                poisoned: final(self)@.poisoned, ..old(self)@ }),
            result.is_ok() ==> final(self)@.accepted && !final(self)@.poisoned,
            result.is_err() && final(self)@.accepted ==> final(self)@.poisoned,
    ))]
    fn append(&mut self) -> Result<(), Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(
        requires commit_stage(old(self)@, 2),
        ensures final(self)@ == (CommitView { poisoned: true, ..old(self)@ }),
    ))]
    fn arm_apply(&mut self);

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires commit_safe(old(self)@), old(self)@.accepted, old(self)@.poisoned,
            !old(self)@.applied, !old(self)@.consumed,
        ensures final(self)@ == (CommitView { applied: final(self)@.applied, ..old(self)@ }),
            result.is_ok() ==> final(self)@.applied,
    ))]
    fn apply(&mut self) -> Result<(), Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(
        requires commit_safe(old(self)@), old(self)@.applied, old(self)@.poisoned,
            !old(self)@.consumed,
        ensures final(self)@ == (CommitView { poisoned: false, ..old(self)@ }),
    ))]
    fn confirm_apply(&mut self);

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires commit_stage(old(self)@, 3),
        ensures final(self)@ == (CommitView { consumed: final(self)@.consumed, ..old(self)@ }),
            result.is_ok() ==> final(self)@.consumed,
    ))]
    fn consume(&mut self) -> Result<(), Self::Error>;
}

/// Check the abstract effect dispatch, including its application guard.
/// An unwind during apply cannot reach confirm_apply. Native adapters must not
/// clear the guard themselves; caught-unwind behavior is also tested directly.
#[cfg_attr(straw_verus, verus_spec(result =>
    requires commit_ready(old(effects)@),
    ensures final(effects)@.request == old(effects)@.request,
        commit_safe(final(effects)@),
        final(effects)@.accepted && !final(effects)@.applied ==> final(effects)@.poisoned,
        result.is_ok() ==> result->Ok_0 && commit_stage(final(effects)@, 4),
))]
#[cfg(straw_verus)]
pub fn execute_commit<E: CommitEffects>(effects: &mut E) -> Result<bool, E::Error> {
    let mut protocol = commit_start();
    #[cfg_attr(straw_verus, verus_spec(
        invariant protocol.completed <= 4, !protocol.failed,
            effects@.request == old(effects)@.request,
            commit_stage(effects@, protocol.completed as nat),
        decreases 4 - protocol.completed,
    ))]
    loop {
        let effect = match commit_next(&protocol) {
            Some(effect) => effect,
            None => return Ok(commit_finished(&protocol)),
        };
        let outcome = match effect {
            CommitEffect::Retain => effects.retain(),
            CommitEffect::AppendWal => effects.append(),
            CommitEffect::Apply => {
                effects.arm_apply();
                let applied = effects.apply();
                if applied.is_ok() {
                    effects.confirm_apply();
                }
                applied
            }
            CommitEffect::ConsumeStaging => effects.consume(),
        };
        protocol = commit_advance(protocol, outcome.is_ok());
        outcome?;
    }
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteEffect {
    Unlink,
    SyncParent,
}
#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug)]
pub struct DeleteProtocol {
    pub completed: u8,
    pub failed: bool,
    pub tombstoned: bool,
}
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result@ == initial(), result.tombstoned == tombstoned,
))]
pub fn delete_start(tombstoned: bool) -> DeleteProtocol {
    DeleteProtocol {
        completed: 0,
        failed: false,
        tombstoned,
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == delete_effect(state@, state.tombstoned),
        result.is_some() ==> state.tombstoned,
))]
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

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.tombstoned == state.tombstoned,
        result@ == if state.tombstoned { advance(state@, success, 2) } else { state@ },
        state@.completed <= 2 ==> result@.completed <= 2,
))]
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

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (state.tombstoned && !state@.failed && state@.completed == 2),
))]
pub fn delete_finished(state: &DeleteProtocol) -> bool {
    state.tombstoned && !state.failed && state.completed == 2
}

/// Complete a batch only after every parent barrier succeeded. Each member
/// must be waiting for that barrier; an already-finished or failed member is
/// rejected as well. Directory/path coverage belongs to the I/O adapter.
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (synced && forall|i: int| 0 <= i < states.len() ==> waiting_for_sync(states[i])),
))]
pub fn delete_batch_finished(states: &[DeleteProtocol], synced: bool) -> bool {
    if !synced {
        return false;
    }
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant
            i <= states.len(),
            forall|k: int| 0 <= k < i ==> waiting_for_sync(states[k]),
        decreases states.len() - i,
    ))]
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

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug)]
pub enum ReclaimEffect {
    SyncParents,
    RecordReceipt,
}
#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug)]
pub struct ReclaimProtocol {
    pub completed: u8,
    pub failed: bool,
}
#[cfg_attr(straw_verus, verus_spec(result => ensures result@ == initial()))]
pub fn reclaim_start() -> ReclaimProtocol {
    ReclaimProtocol {
        completed: 0,
        failed: false,
    }
}
#[cfg_attr(straw_verus, verus_spec(result => ensures result == reclaim_effect(state@)))]
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
#[cfg_attr(straw_verus, verus_spec(result => ensures result@ == advance(state@, success, 2)))]
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
#[cfg_attr(straw_verus, verus_spec(result => ensures result == (!state@.failed && state@.completed == 2)))]
pub fn reclaim_finished(state: &ReclaimProtocol) -> bool {
    !state.failed && state.completed == 2
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug)]
pub struct PackStatus {
    pub sealed: bool,
    pub owned: bool,
    pub tombstoned: bool,
}

/// Final authorization uses a fresh locked snapshot, not the pre-scan one.
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (verified && statuses.len() > 0 &&
        forall|i: int| 0 <= i < statuses.len() ==> statuses[i].owned && !statuses[i].tombstoned),
))]
pub fn reactivation_permitted(verified: bool, statuses: &[PackStatus]) -> bool {
    if !verified || statuses.is_empty() {
        return false;
    }
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant
            i <= statuses.len(),
            forall|k: int| 0 <= k < i ==> statuses[k].owned && !statuses[k].tombstoned,
        decreases statuses.len() - i,
    ))]
    while i < statuses.len() {
        if !statuses[i].owned || statuses[i].tombstoned {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == eligible(status),
))]
pub fn candidate(status: PackStatus) -> bool {
    status.sealed && !status.owned && !status.tombstoned
}

#[cfg(straw_verus)]
verus! {
    pub struct ProgressView { pub completed: nat, pub failed: bool }
    pub open spec fn initial() -> ProgressView { ProgressView { completed: 0, failed: false } }
    pub open spec fn advance(state: ProgressView, success: bool, limit: nat) -> ProgressView {
        if state.failed || state.completed >= limit { state }
        else if success { ProgressView { completed: state.completed + 1, failed: false } }
        else { ProgressView { completed: state.completed, failed: true } }
    }
    impl View for CommitProtocol {
        type V = ProgressView;
        open spec fn view(&self) -> ProgressView { ProgressView { completed: self.completed as nat, failed: self.failed } }
    }
    impl View for DeleteProtocol {
        type V = ProgressView;
        open spec fn view(&self) -> ProgressView { ProgressView { completed: self.completed as nat, failed: self.failed } }
    }
    impl View for ReclaimProtocol {
        type V = ProgressView;
        open spec fn view(&self) -> ProgressView { ProgressView { completed: self.completed as nat, failed: self.failed } }
    }
    pub open spec fn reclaim_effect(state: ProgressView) -> Option<ReclaimEffect> {
        if state.failed { None }
        else if state.completed == 0 { Some(ReclaimEffect::SyncParents) }
        else if state.completed == 1 { Some(ReclaimEffect::RecordReceipt) }
        else { None }
    }
    pub open spec fn commit_effect(state: ProgressView) -> Option<CommitEffect> {
        if state.failed { None }
        else if state.completed == 0 { Some(CommitEffect::Retain) }
        else if state.completed == 1 { Some(CommitEffect::AppendWal) }
        else if state.completed == 2 { Some(CommitEffect::Apply) }
        else if state.completed == 3 { Some(CommitEffect::ConsumeStaging) }
        else { None }
    }
    pub open spec fn delete_effect(state: ProgressView, tombstoned: bool) -> Option<DeleteEffect> {
        if state.failed || !tombstoned { None }
        else if state.completed == 0 { Some(DeleteEffect::Unlink) }
        else if state.completed == 1 { Some(DeleteEffect::SyncParent) }
        else { None }
    }
    pub open spec fn waiting_for_sync(state: DeleteProtocol) -> bool {
        state.tombstoned && !state@.failed && state@.completed == 1
    }
    pub open spec fn eligible(status: PackStatus) -> bool { status.sealed && !status.owned && !status.tombstoned }
}
