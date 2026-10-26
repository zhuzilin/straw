//! The actual catalog indexes, replay cursor and confirmation state.
//! I/O establishes durability; this state accepts only matching boundaries.
use crate::catalog_state::{Indexes, Operation};
use crate::{replay, wal};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Unavailable,
    Truncated,
    Boundary,
    InvalidOperation(usize),
    Mismatch,
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum AppendFailure<E> {
    State(Failure),
    Write(crate::wal_append::Failure<E>),
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Pending {
    operations: Vec<Operation>,
    end: u64,
    sequence: u64,
    #[cfg(straw_verus)]
    payload: Ghost<Seq<u8>>,
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct LogState {
    indexes: Indexes,
    offset: u64,
    sequence: u64,
    recovery: replay::Recovery,
    complete: bool,
    pending: Option<Pending>,
    #[cfg(straw_verus)]
    history: Ghost<Seq<Record>>,
    #[cfg(straw_verus)]
    partial: Ghost<Seq<Operation>>,
}

#[cfg(straw_verus)]
verus! {
    pub struct Record {
        pub operations: Seq<Operation>,
        pub payload: Seq<u8>,
    }
    pub open spec fn initial_indexes() -> crate::catalog_state::State {
        crate::catalog_state::State {
            owners: Map::empty(), packs: Map::empty(), deleted: Set::empty(),
            pending: Set::empty(), edges: Map::empty(), receipts: Set::empty(),
        }
    }
    pub closed spec fn history_state(h: Seq<Record>) -> crate::catalog_state::State
        decreases h.len(),
    {
        if h.len() == 0 { initial_indexes() }
        else { crate::catalog_state::run(history_state(h.drop_last()), h.last().operations) }
    }
    pub proof fn history_unfold(h: Seq<Record>)
        ensures history_state(h) == if h.len() == 0 { initial_indexes() }
            else { crate::catalog_state::run(history_state(h.drop_last()), h.last().operations) },
    { reveal(history_state); }
    pub closed spec fn history_bytes(h: Seq<Record>) -> nat
        decreases h.len(),
    {
        if h.len() == 0 { 0 }
        else { history_bytes(h.drop_last()) + h.last().payload.len() + 96 }
    }
    pub closed spec fn history_accepted(h: Seq<Record>) -> bool
        decreases h.len(),
    {
        h.len() == 0 || (history_accepted(h.drop_last())
            && crate::catalog_state::accepted(history_state(h.drop_last()), h.last().operations))
    }
    pub proof fn extend_history(h: Seq<Record>, r: Record)
        requires history_accepted(h), crate::catalog_state::accepted(history_state(h), r.operations),
        ensures history_accepted(h.push(r)),
            history_state(h.push(r)) == crate::catalog_state::run(history_state(h), r.operations),
            history_bytes(h.push(r)) == history_bytes(h) + r.payload.len() + 96,
    {
        reveal(history_accepted); reveal(history_state); reveal(history_bytes);
        assert(h.push(r).drop_last() == h);
    }
    pub open spec fn payloads(h: Seq<Record>) -> Seq<Seq<u8>> {
        Seq::new(h.len(), |i: int| h[i].payload)
    }
    pub proof fn payloads_push(h: Seq<Record>, r: Record)
        ensures payloads(h.push(r)) == payloads(h).push(r.payload),
    {
        assert(payloads(h.push(r)) =~= payloads(h).push(r.payload));
    }
    /// Byte lengths depend on payloads, independently of decoded operations.
    pub proof fn payload_bytes(h: Seq<Record>)
        ensures history_bytes(h) == crate::wal_recovery::run_end(0, payloads(h)),
        decreases h.len(),
    {
        reveal(history_bytes); reveal(crate::wal_recovery::run_end);
        if h.len() > 0 {
            payload_bytes(h.drop_last());
            assert(payloads(h).drop_last() =~= payloads(h.drop_last()));
        }
    }
    pub proof fn suffix_boundary(records: Seq<Seq<u8>>, split: nat)
        requires split <= records.len(),
        ensures crate::wal_recovery::run_end(crate::wal_recovery::run_end(0, records.take(split as int)),
            records.subrange(split as int, records.len() as int)) == crate::wal_recovery::run_end(0, records),
        decreases records.len() - split,
    {
        if split == records.len() {
            assert(records.take(split as int) =~= records);
            assert(records.subrange(split as int, records.len() as int) =~= Seq::<Seq<u8>>::empty());
            reveal(crate::wal_recovery::run_end);
        } else {
            suffix_boundary(records.drop_last(), split);
            assert(records.drop_last().take(split as int) =~= records.take(split as int));
            assert(records.subrange(split as int, records.len() as int).drop_last()
                =~= records.drop_last().subrange(split as int, records.len() as int - 1));
            reveal(crate::wal_recovery::run_end);
        }
    }
    pub proof fn payload_boundary(previous: Seq<Record>, current: Seq<Record>)
        requires previous.len() <= current.len(),
            payloads(current).take(previous.len() as int) == payloads(previous),
        ensures history_bytes(current) == crate::wal_recovery::run_end(history_bytes(previous) as int,
            payloads(current).subrange(previous.len() as int, current.len() as int)),
    {
        payload_bytes(previous); payload_bytes(current);
        suffix_boundary(payloads(current), previous.len());
    }
    pub struct Plan {
        pub operations: Seq<Operation>,
        pub end: u64,
        pub sequence: u64,
        pub payload: Seq<u8>,
    }
    impl View for Pending {
        type V = Plan;
        closed spec fn view(&self) -> Plan {
            Plan { operations: self.operations@, end: self.end, sequence: self.sequence, payload: self.payload@ }
        }
    }
    pub struct State {
        pub indexes: crate::catalog_state::State,
        pub offset: u64,
        pub sequence: u64,
        pub floor: u64,
        pub dirty: bool,
        pub complete: bool,
        pub pending: Option<Plan>,
        pub history: Seq<Record>,
        pub partial: Seq<Operation>,
    }
    impl View for LogState {
        type V = State;
        closed spec fn view(&self) -> State {
            State { indexes: self.indexes@, offset: self.offset, sequence: self.sequence,
                floor: self.recovery.durable, dirty: self.recovery.rebuild,
                complete: self.complete, pending: match self.pending {
                    None => None, Some(p) => Some(p@),
                }, history: self.history@, partial: self.partial@ }
        }
    }
    pub open spec fn valid(s: State) -> bool {
        crate::catalog_state::valid(s.indexes)
        && history_accepted(s.history)
        && s.indexes == crate::catalog_state::run(history_state(s.history), s.partial)
        && (s.complete ==> s.partial.len() == 0)
        && s.sequence == s.history.len() && s.offset == history_bytes(s.history)
        && s.sequence as int * 96 <= s.offset as int
        && (!s.dirty ==> s.complete && s.pending.is_none() && s.floor == s.offset)
        && match s.pending {
            None => true,
            Some(p) => s.dirty && s.complete && s.floor == s.offset
                && p.end as int == s.offset as int + p.payload.len() + 96
                && p.sequence as int == s.sequence as int + 1
                && crate::catalog_state::accepted(s.indexes, p.operations),
        }
    }
    pub open spec fn empty(s: crate::catalog_state::State) -> bool {
        s.owners == Map::<String, Set<String>>::empty()
            && s.packs == Map::<String, bool>::empty()
            && s.deleted == Set::<String>::empty() && s.pending == Set::<String>::empty()
            && s.edges == Map::<String, Seq<String>>::empty() && s.receipts == Set::<String>::empty()
    }
    pub open spec fn at_boundary(s: State, device: crate::wal_device::DeviceView) -> bool {
        device.cursor == s.offset && device.bytes.len() == s.offset && device.durable == s.offset
    }
    pub open spec fn ready(s: State) -> bool { !s.dirty && s.complete && s.pending.is_none() }
    pub proof fn complete_history(s: State)
        requires valid(s), s.complete,
        ensures s.indexes == history_state(s.history), s.partial == Seq::<Operation>::empty(),
    {
        assert(s.partial =~= Seq::<Operation>::empty());
        crate::catalog_state::run_empty(history_state(s.history));
    }

}

#[cfg_attr(straw_verus, verus_verify)]
impl LogState {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::environment::string_hash_contract(),
        ensures valid(result@), ready(result@), empty(result@.indexes),
            result@.offset == 0, result@.sequence == 0, result@.floor == 0,
            result@.history.len() == 0, result@.partial.len() == 0,
            forall|context: crate::catalog_frame::Context| crate::catalog_target::interpreted(context, result@.history),
    ))]
    pub fn new() -> Self {
        #[cfg(straw_verus)]
        proof! {
            reveal(history_state); reveal(history_bytes); reveal(history_accepted);
            assert forall|context: crate::catalog_frame::Context|
                crate::catalog_target::interpreted(context, Seq::empty()) by {
                crate::catalog_target::empty_interpretation(context);
            }
            crate::catalog_state::run_empty(initial_indexes());
        }
        Self {
            indexes: Indexes::new(),
            offset: 0,
            sequence: 0,
            recovery: replay::start(),
            complete: true,
            pending: None,
            #[cfg(straw_verus)]
            history: verus_exec_expr!(Ghost(Seq::empty())),
            #[cfg(straw_verus)]
            partial: verus_exec_expr!(Ghost(Seq::empty())),
        }
    }

    #[cfg_attr(straw_verus, verus_spec(result => ensures result@ == self@.indexes))]
    pub fn indexes(&self) -> &Indexes {
        &self.indexes
    }

    #[cfg_attr(straw_verus, verus_spec(result => ensures result == self@.offset))]
    pub fn offset(&self) -> u64 {
        self.offset
    }

    #[cfg_attr(straw_verus, verus_spec(result => ensures result == self@.sequence))]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    #[cfg_attr(straw_verus, verus_spec(result => ensures result == self@.floor))]
    pub fn floor(&self) -> u64 {
        self.recovery.durable
    }

    #[cfg_attr(straw_verus, verus_spec(result => ensures result == self@.dirty))]
    pub fn dirty(&self) -> bool {
        self.recovery.rebuild
    }

    #[cfg_attr(straw_verus, verus_spec(result => ensures result == ready(self@)))]
    pub fn ready(&self) -> bool {
        replay::ready(&self.recovery) && self.complete && self.pending.is_none()
    }

    #[cfg_attr(straw_verus, verus_spec(
        requires valid(old(self)@),
        ensures valid(final(self)@), final(self)@ == (State { dirty: true, ..old(self)@ }),
    ))]
    pub fn invalidate(&mut self) {
        replay::prepare(&mut self.recovery, true);
    }

    /// Return whether derived namespace/closure caches must also be cleared.
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires valid(old(self)@), crate::environment::string_hash_contract(),
        ensures valid(final(self)@), final(self)@.floor == old(self)@.floor,
            result.is_ok() == (size >= old(self)@.floor
                && (old(self)@.dirty || size >= old(self)@.offset)),
            result.is_err() ==> final(self)@ == old(self)@ && result == Err(Failure::Truncated),
            result.is_ok() ==> result->Ok_0 == old(self)@.dirty,
            result.is_ok() ==> final(self)@.offset <= size && final(self)@.floor <= size
                && final(self)@.complete && final(self)@.pending.is_none(),
            result.is_ok() && result->Ok_0 ==> empty(final(self)@.indexes)
                && final(self)@.offset == 0 && final(self)@.sequence == 0 && final(self)@.dirty
                && final(self)@.history.len() == 0 && final(self)@.partial.len() == 0,
            result.is_ok() && !result->Ok_0 ==> final(self)@ == (State {
                dirty: old(self)@.offset < size, ..old(self)@ }),
    ))]
    pub fn begin_replay(&mut self, size: u64) -> Result<bool, Failure> {
        if !replay::accepts(&self.recovery, size) || (!self.recovery.rebuild && size < self.offset)
        {
            return Err(Failure::Truncated);
        }
        let rebuilt = self.dirty();
        if rebuilt {
            self.indexes = Indexes::new();
            self.offset = 0;
            self.sequence = 0;
            self.complete = true;
            self.pending = None;
            #[cfg(straw_verus)]
            proof! {
                self.history = Ghost(Seq::empty());
                self.partial = Ghost(Seq::empty());
                reveal(history_state); reveal(history_bytes); reveal(history_accepted);
                crate::catalog_state::run_empty(initial_indexes());
            }
        }
        replay::prepare(&mut self.recovery, self.offset < size);
        Ok(rebuilt)
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires valid(old(self)@), crate::environment::string_hash_contract(),
            crate::environment::string_order_contract(),
        ensures valid(final(self)@), final(self)@.floor == old(self)@.floor,
            result.is_ok() == (old(self)@.complete && old(self)@.pending.is_none()
                && old(self)@.sequence < u64::MAX
                && old(self)@.offset as int + payload.len() + 96 <= u64::MAX as int
                && crate::catalog_state::accepted(old(self)@.indexes, ops@)),
            result.is_ok() ==> final(self)@.indexes == crate::catalog_state::run(old(self)@.indexes, ops@)
                && crate::catalog_state::accepted(old(self)@.indexes, ops@)
                && final(self)@.offset as int == old(self)@.offset as int + payload.len() + 96
                && final(self)@.sequence as int == old(self)@.sequence as int + 1
                && final(self)@.dirty && final(self)@.complete && final(self)@.pending.is_none()
                && final(self)@.history == old(self)@.history.push(Record { operations: ops@, payload: payload@ })
                && final(self)@.partial.len() == 0,
            result.is_err() && !(result->Err_0 is InvalidOperation) ==> final(self)@ == old(self)@,
            result.is_err() && result->Err_0 is InvalidOperation ==> final(self)@.dirty && !final(self)@.complete
                && final(self)@.pending.is_none() && final(self)@.offset == old(self)@.offset
                && final(self)@.sequence == old(self)@.sequence
                && final(self)@.history == old(self)@.history
                && match result->Err_0 { Failure::InvalidOperation(index) => index < ops.len()
                    && final(self)@.indexes == crate::catalog_state::run(old(self)@.indexes, ops@.take(index as int))
                    && final(self)@.partial == ops@.take(index as int),
                    _ => false },
    ))]
    pub fn replay_record(&mut self, ops: &[Operation], payload: &[u8]) -> Result<(), Failure> {
        if !self.complete || self.pending.is_some() {
            return Err(Failure::Unavailable);
        }
        let end = frame_end(self.offset, self.sequence, payload.len() as u64)?;
        #[cfg(straw_verus)]
        proof! { complete_history(self@); }
        self.invalidate();
        self.complete = false;
        if let Err(index) = self.indexes.apply(ops) {
            #[cfg(straw_verus)]
            proof! { self.partial = Ghost(ops@.take(index as int)); }
            return Err(Failure::InvalidOperation(index));
        }
        #[cfg(straw_verus)]
        proof! {
            let record = Record { operations: ops@, payload: payload@ };
            extend_history(self.history@, record);
            self.history = Ghost(self.history@.push(record));
            crate::catalog_state::run_empty(history_state(self.history@));
        }
        self.offset = end;
        self.sequence += 1;
        self.complete = true;
        Ok(())
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires valid(old(self)@),
        ensures valid(final(self)@),
            result.is_err() ==> final(self)@ == old(self)@,
            result.is_ok() ==> final(self)@ == (State { floor: end, dirty: false, ..old(self)@ })
                && final(self)@.offset == end && final(self)@.sequence == sequence && ready(final(self)@),
            result.is_ok() == (old(self)@.complete && old(self)@.pending.is_none()
                && end == old(self)@.offset && sequence == old(self)@.sequence && end >= old(self)@.floor),
    ))]
    pub fn confirm_replay(&mut self, end: u64, sequence: u64) -> Result<(), Failure> {
        if !self.complete
            || self.pending.is_some()
            || end != self.offset
            || sequence != self.sequence
        {
            return Err(Failure::Mismatch);
        }
        if !replay::confirm(&mut self.recovery, end) {
            return Err(Failure::Truncated);
        }
        Ok(())
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires valid(old(self)@), crate::environment::string_hash_contract(),
        ensures valid(final(self)@),
            result.is_ok() == (ready(old(self)@) && old(self)@.sequence < u64::MAX
                && old(self)@.offset as int + payload.len() + 96 <= u64::MAX as int
                && crate::catalog_state::accepted(old(self)@.indexes, ops@)),
            result.is_err() ==> final(self)@ == old(self)@,
            result.is_ok() ==> final(self)@ == (State { dirty: true,
                pending: Some(Plan { operations: ops@,
                    end: (old(self)@.offset as int + payload.len() + 96) as u64,
                    sequence: (old(self)@.sequence + 1) as u64, payload: payload@ }), ..old(self)@ }),
            result.is_ok() ==> ready(old(self)@),
    ))]
    pub fn begin_append(&mut self, ops: Vec<Operation>, payload: &[u8]) -> Result<(), Failure> {
        if !self.ready() {
            return Err(Failure::Unavailable);
        }
        let end = frame_end(self.offset, self.sequence, payload.len() as u64)?;
        if let Err(index) = self.indexes.validate(&ops) {
            return Err(Failure::InvalidOperation(index));
        }
        self.invalidate();
        self.pending = Some(Pending {
            operations: ops,
            end,
            sequence: self.sequence + 1,
            #[cfg(straw_verus)]
            payload: verus_exec_expr!(Ghost(payload@)),
        });
        Ok(())
    }

    /// Compose admission, the actual byte append driver and confirmation.
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires valid(old(self)@), old(device).valid(), !old(device)@.buffered,
            crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
        ensures final(device).loan() == old(device).loan(), valid(final(self)@), final(device).valid(), final(device)@.identity == old(device)@.identity,
            !final(device)@.buffered,
            result.is_ok() ==> ready(final(self)@)
                && final(self)@.indexes == crate::catalog_state::run(old(self)@.indexes, operations@)
                && final(self)@.offset as int == old(self)@.offset as int + payload.len() + 96
                && final(self)@.sequence as int == old(self)@.sequence as int + 1
                && final(self)@.floor == final(self)@.offset
                && final(device)@.bytes.len() == final(self)@.offset
                && final(device)@.cursor == final(self)@.offset
                && final(device)@.durable == final(self)@.offset,
            result.is_ok() ==> final(self)@.history == old(self)@.history.push(Record { operations: operations@, payload: payload@ }),
            result.is_ok() ==> ready(old(self)@),
            result.is_err() ==> final(self)@.history == old(self)@.history && final(self)@.indexes == old(self)@.indexes,
            result.is_ok() ==> crate::wal_recovery::complete_frame(crate::framing::Kind::Catalog,
                final(device)@.bytes, old(self)@.offset as int, old(self)@.sequence as nat,
                payload@, final(self)@.offset as int),
            result.is_err() && result->Err_0 is State ==> final(self)@ == old(self)@
                && final(device)@ == old(device)@,
            result.is_err() && result->Err_0 is Write ==> final(self)@.dirty
                && final(self)@.pending.is_some() && !ready(final(self)@)
                && final(self)@.offset == old(self)@.offset
                && final(self)@.sequence == old(self)@.sequence
                && final(self)@.floor == old(self)@.floor,
    ))]
    pub fn append_to<D: crate::wal_device::Writer>(
        &mut self,
        device: &mut D,
        operations: Vec<Operation>,
        payload: &[u8],
    ) -> Result<(), AppendFailure<D::Error>> {
        let offset = self.offset();
        let sequence = self.sequence();
        if let Err(error) = self.begin_append(operations, payload) {
            return Err(AppendFailure::State(error));
        }
        if let Err(error) = crate::wal_append::append(
            device,
            crate::framing::Kind::Catalog,
            sequence,
            offset,
            payload,
        ) {
            return Err(AppendFailure::Write(error));
        }
        match self.finish_append() {
            Ok(()) => Ok(()),
            Err(error) => Err(AppendFailure::State(error)),
        }
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires valid(old(self)@), crate::environment::string_hash_contract(),
            crate::environment::string_order_contract(),
        ensures valid(final(self)@),
            result.is_ok() == old(self)@.pending.is_some(),
            result.is_err() ==> final(self)@ == old(self)@,
            result.is_ok() ==> final(self)@ == (State {
                indexes: crate::catalog_state::run(old(self)@.indexes, old(self)@.pending.unwrap().operations),
                offset: old(self)@.pending.unwrap().end, sequence: old(self)@.pending.unwrap().sequence,
                floor: old(self)@.pending.unwrap().end, dirty: false, complete: true, pending: None,
                history: old(self)@.history.push(Record {
                    operations: old(self)@.pending.unwrap().operations,
                    payload: old(self)@.pending.unwrap().payload }),
                partial: Seq::empty() }),
    ))]
    pub fn finish_append(&mut self) -> Result<(), Failure> {
        #[cfg(straw_verus)]
        proof! { if self.pending.is_some() { complete_history(self@); } }
        let Some(plan) = self.pending.take() else {
            return Err(Failure::Unavailable);
        };
        self.complete = false;
        if let Err(index) = self.indexes.apply(&plan.operations) {
            return Err(Failure::InvalidOperation(index));
        }
        #[cfg(straw_verus)]
        proof! {
            let record = Record { operations: plan.operations@,
                payload: plan.payload@ };
            extend_history(self.history@, record);
            self.history = Ghost(self.history@.push(record));
            crate::catalog_state::run_empty(history_state(self.history@));
        }
        self.offset = plan.end;
        self.sequence = plan.sequence;
        self.complete = true;
        if !replay::confirm(&mut self.recovery, plan.end) {
            return Err(Failure::Truncated);
        }
        Ok(())
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == (sequence < u64::MAX
            && offset as int + payload as int + 96 <= u64::MAX as int),
        result.is_ok() ==> result->Ok_0 as int == offset as int + payload as int + 96
        && sequence < u64::MAX,
        result.is_err() ==> result == Err(Failure::Boundary),
))]
fn frame_end(offset: u64, sequence: u64, payload: u64) -> Result<u64, Failure> {
    if sequence == u64::MAX {
        return Err(Failure::Boundary);
    }
    match wal::frame_extent(offset, u64::MAX, payload) {
        wal::FrameExtent::Complete { end } => Ok(end),
        _ => Err(Failure::Boundary),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn owner(name: &str) -> Operation {
        Operation::Owner(name.into(), BTreeSet::from(["raw/pack".into()]))
    }

    #[test]
    fn partial_apply_cannot_be_confirmed_or_continued() {
        let mut state = LogState::new();
        assert_eq!(state.begin_replay(200), Ok(false));
        assert_eq!(
            state.replay_record(
                &[owner("tentative"), Operation::Reclaim("missing".into())],
                &[0; 8]
            ),
            Err(Failure::InvalidOperation(1))
        );
        assert!(state.indexes().owners.contains_key("tentative"));
        assert_eq!(state.offset(), 0);
        assert_eq!(state.sequence(), 0);
        assert_eq!(state.floor(), 0);
        assert!(state.dirty());
        assert_eq!(state.confirm_replay(0, 0), Err(Failure::Mismatch));
        assert_eq!(
            state.replay_record(&[owner("later")], &[0; 8]),
            Err(Failure::Unavailable)
        );
        assert_eq!(
            state.begin_append(vec![owner("later")], &[0; 8]),
            Err(Failure::Unavailable)
        );
        assert_eq!(state.begin_replay(0), Ok(true));
        assert!(state.indexes().owners.is_empty());
        assert_eq!(state.confirm_replay(0, 0), Ok(()));
        assert!(state.ready());
    }

    #[test]
    fn uncertain_append_plan_is_discarded_by_recovery() {
        let mut state = LogState::new();
        assert_eq!(state.begin_append(vec![owner("old-plan")], &[0; 8]), Ok(()));
        assert!(!state.ready());
        assert!(state.indexes().owners.is_empty());
        assert_eq!(
            state.begin_append(vec![owner("later")], &[0; 8]),
            Err(Failure::Unavailable)
        );
        assert_eq!(state.confirm_replay(0, 0), Err(Failure::Mismatch));
        assert_eq!(state.begin_replay(104), Ok(true));
        assert_eq!(state.finish_append(), Err(Failure::Unavailable));
        assert_eq!(state.replay_record(&[owner("replayed")], &[0; 8]), Ok(()));
        assert_eq!(state.confirm_replay(103, 1), Err(Failure::Mismatch));
        assert_eq!(state.confirm_replay(104, 2), Err(Failure::Mismatch));
        assert_eq!(state.floor(), 0);
        assert_eq!(state.confirm_replay(104, 1), Ok(()));
        assert!(state.ready());
        assert!(state.indexes().owners.contains_key("replayed"));
        assert!(!state.indexes().owners.contains_key("old-plan"));
    }

    #[test]
    fn append_applies_the_stored_operations_once() {
        let mut state = LogState::new();
        assert_eq!(
            state.begin_append(
                vec![Operation::Pack("raw/pack".into(), false), owner("reader")],
                &[0; 7]
            ),
            Ok(())
        );
        assert_eq!(state.finish_append(), Ok(()));
        assert_eq!(
            (state.offset(), state.sequence(), state.floor()),
            (103, 1, 103)
        );
        assert!(state.ready());
        assert!(state.indexes().owners.contains_key("reader"));
        assert_eq!(state.indexes().packs.get("raw/pack"), Some(&false));
        assert_eq!(state.finish_append(), Err(Failure::Unavailable));
        assert_eq!(state.begin_replay(102), Err(Failure::Truncated));
        assert_eq!(
            (state.offset(), state.sequence(), state.floor()),
            (103, 1, 103)
        );
        assert_eq!(state.begin_replay(103), Ok(false));
        assert_eq!(state.confirm_replay(103, 1), Ok(()));
    }

    #[test]
    fn failed_admission_and_overflow_leave_the_confirmed_state_usable() {
        let mut state = LogState::new();
        assert_eq!(
            state.begin_append(vec![Operation::Reclaim("missing".into())], &[0; 8]),
            Err(Failure::InvalidOperation(0))
        );
        assert_eq!(frame_end(0, 0, u64::MAX), Err(Failure::Boundary));
        assert_eq!(frame_end(1, 0, u64::MAX - 96), Err(Failure::Boundary));
        assert!(state.ready());
        assert_eq!((state.offset(), state.sequence(), state.floor()), (0, 0, 0));
        assert!(state.indexes().owners.is_empty());
    }
}
