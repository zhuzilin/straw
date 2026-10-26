//! The production batch deletion dispatch. Native effects must bind each index
//! to its durable tombstone and stable parent directory; errors may have effects.
use crate::protocol::{self, DeleteEffect, ReclaimEffect};
use std::collections::BTreeSet;
#[cfg(straw_verus)]
use vstd::{prelude::*, std_specs::iter::IteratorSpec};

#[cfg(straw_verus)]
verus! {
    pub struct BatchView<P> {
        pub parents: Seq<P>,
        pub marked: ISet<nat>,
        pub absent: ISet<nat>,
        pub durable_absent: ISet<nat>,
        pub receipted: ISet<nat>,
    }
    pub trait BatchModel: View<V = BatchView<Self::Parent>> { type Parent: Ord; type Context; #[verifier::prophetic] spec fn context(&self) -> Self::Context; open spec fn context_valid(&self) -> bool { true } open spec fn confirmed(&self) -> bool { true } }
    pub open spec fn indices<P>(s: BatchView<P>) -> ISet<nat> {
        ISet::new(|i: nat| i < s.parents.len())
    }
    pub open spec fn valid<P>(s: BatchView<P>) -> bool {
        s.receipted.subset_of(s.durable_absent) && s.durable_absent.subset_of(s.absent)
            && s.absent.subset_of(indices(s)) && s.marked.subset_of(indices(s))
            && s.absent.subset_of(s.marked)
    }
    pub open spec fn same_batch<P>(a: BatchView<P>, b: BatchView<P>) -> bool {
        a.parents == b.parents && a.marked == b.marked
    }
    pub open spec fn under<P>(s: BatchView<P>, parents: Set<P>) -> ISet<nat> {
        ISet::new(|i: nat| i < s.parents.len() && parents.contains(s.parents[i as int]))
    }
}
#[cfg(not(straw_verus))]
pub trait BatchModel {
    type Parent: Ord;
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Removal<P> {
    pub parent: P,
    /// Some means unlink removed a name; None is an unlink NotFound response.
    pub bytes: Option<u64>,
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct Statistics {
    pub files: u64,
    pub bytes: u64,
}
#[cfg_attr(straw_verus, verus_verify)]
pub enum Failure<E> {
    Effect(E),
    Protocol,
    Accounting,
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Effects: BatchModel {
    type Error;
    #[cfg_attr(straw_verus, verus_spec(result => requires self.context_valid(), ensures result == self@.parents.len()))]
    fn target_count(&self) -> usize;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self.context_valid(), index < self@.parents.len(),
        ensures result == self@.marked.contains(index as nat),
    ))]
    fn marked(&self, index: usize) -> bool;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).context_valid(), valid(old(self)@), index < old(self)@.parents.len(), old(self)@.marked.contains(index as nat),
        ensures final(self).context_valid(), final(self).context() == old(self).context(), valid(final(self)@), same_batch(old(self)@, final(self)@),
            final(self)@.receipted == old(self)@.receipted,
            old(self)@.durable_absent.subset_of(final(self)@.durable_absent),
            result.is_ok() ==> old(self)@.absent.subset_of(final(self)@.absent)
                && final(self)@.absent.contains(index as nat)
                && result->Ok_0.parent == old(self)@.parents[index as int],
    ))]
    fn unlink(&mut self, index: usize) -> Result<Removal<Self::Parent>, Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).context_valid(), valid(old(self)@), old(self)@.absent == indices(old(self)@),
        ensures final(self).context_valid(), final(self).context() == old(self).context(), valid(final(self)@), same_batch(old(self)@, final(self)@),
            final(self)@.receipted == old(self)@.receipted,
            old(self)@.durable_absent.subset_of(final(self)@.durable_absent),
            result.is_ok() ==> final(self)@.absent == old(self)@.absent
                && under(old(self)@, Set::empty().insert(*parent)).subset_of(final(self)@.durable_absent),
    ))]
    fn sync_parent(&mut self, parent: &Self::Parent) -> Result<(), Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).context_valid(), valid(old(self)@), old(self)@.durable_absent == indices(old(self)@),
        ensures final(self).context_valid(), final(self).context() == old(self).context(), same_batch(old(self)@, final(self)@),
            final(self)@.receipted.subset_of(indices(final(self)@)),
            final(self)@.absent == old(self)@.absent,
            final(self)@.durable_absent == old(self)@.durable_absent,
            old(self)@.receipted.subset_of(final(self)@.receipted),
            result.is_ok() ==> final(self).confirmed() && final(self)@.receipted == indices(final(self)@),
    ))]
    fn receipt(&mut self) -> Result<(), Self::Error>;
}

/// Deduplicate parent barriers only after every target has been unlinked.
/// Already-absent names still contribute their parent; a receipt may survive
/// an error response, so its precondition requires durable absence already.
#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(effects).context_valid(), valid(old(effects)@), vstd::laws_cmp::obeys_cmp::<E::Parent>(),
    ensures final(effects).context_valid(), final(effects).context() == old(effects).context(), valid(final(effects)@), same_batch(old(effects)@, final(effects)@),
        old(effects)@.receipted.subset_of(final(effects)@.receipted),
        old(effects)@.durable_absent.subset_of(final(effects)@.durable_absent),
        result == Err(Failure::Protocol) ==> old(effects)@.marked != indices(old(effects)@),
        result.is_ok() ==> final(effects).confirmed() && final(effects)@.durable_absent == indices(final(effects)@)
            && final(effects)@.receipted == indices(final(effects)@),
))]
pub fn collect<E: Effects>(effects: &mut E) -> Result<Statistics, Failure<E::Error>> {
    let count = effects.target_count();
    let mut parents = BTreeSet::new();
    let mut pending = Vec::new();
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut index = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant effects.context_valid(), effects.context() == old(effects).context(), index <= count, count == effects@.parents.len(), valid(effects@),
            vstd::laws_cmp::obeys_cmp::<E::Parent>(),
            same_batch(old(effects)@, effects@), effects@.receipted == old(effects)@.receipted,
            old(effects)@.durable_absent.subset_of(effects@.durable_absent),
            pending.len() == index,
            forall|j: nat| j < index ==> effects@.absent.contains(j),
            forall|j: int| 0 <= j < index ==> parents@.contains(effects@.parents[j]),
            forall|j: int| 0 <= j < index ==> protocol::waiting_for_sync(pending[j]),
        decreases count - index,
    ))]
    while index < count {
        let marked = effects.marked(index);
        let deletion = protocol::delete_start(marked);
        if !matches!(protocol::delete_next(&deletion), Some(DeleteEffect::Unlink)) {
            return Err(Failure::Protocol);
        }
        let removal = match effects.unlink(index) {
            Ok(removal) => removal,
            Err(error) => return Err(Failure::Effect(error)),
        };
        let deletion = protocol::delete_advance(deletion, true);
        parents.insert(removal.parent);
        pending.push(deletion);
        if let Some(size) = removal.bytes {
            files = match crate::bounds::add(files, 1) {
                Some(value) => value,
                None => return Err(Failure::Accounting),
            };
            bytes = match crate::bounds::add(bytes, size) {
                Some(value) => value,
                None => return Err(Failure::Accounting),
            };
        }
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! { assert(effects@.absent =~= indices(effects@)); }
    let mut state = protocol::reclaim_start();
    #[cfg_attr(straw_verus, verus_spec(
        invariant effects.context_valid(), effects.context() == old(effects).context(), state.completed <= 2, !state.failed, valid(effects@),
            vstd::laws_cmp::obeys_cmp::<E::Parent>(),
            same_batch(old(effects)@, effects@), effects@.absent == indices(effects@),
            old(effects)@.durable_absent.subset_of(effects@.durable_absent),
            old(effects)@.receipted.subset_of(effects@.receipted),
            forall|j: int| 0 <= j < effects@.parents.len() ==> parents@.contains(effects@.parents[j]),
            forall|j: int| 0 <= j < pending.len() ==> protocol::waiting_for_sync(pending[j]),
            state.completed >= 1 ==> effects@.durable_absent == indices(effects@),
            state.completed >= 2 ==> effects.confirmed() && effects@.receipted == indices(effects@),
        decreases 2 - state.completed,
    ))]
    loop {
        let effect = match protocol::reclaim_next(&state) {
            Some(effect) => effect,
            None => {
                if !protocol::reclaim_finished(&state) {
                    return Err(Failure::Protocol);
                }
                return Ok(Statistics { files, bytes });
            }
        };
        match effect {
            ReclaimEffect::SyncParents => {
                let mut remaining = parents.iter();
                #[cfg_attr(straw_verus, verus_spec(
                    invariant effects.context_valid(), effects.context() == old(effects).context(), remaining.decrease().is_some(), valid(effects@),
                        same_batch(old(effects)@, effects@), effects@.absent == indices(effects@),
                        old(effects)@.durable_absent.subset_of(effects@.durable_absent),
                        old(effects)@.receipted.subset_of(effects@.receipted),
                        forall|j: int| 0 <= j < effects@.parents.len() ==>
                            remaining.remaining().unref().to_set().contains(effects@.parents[j])
                            || effects@.durable_absent.contains(j as nat),
                    ensures remaining.remaining().len() == 0,
                    decreases remaining.decrease().unwrap(),
                ))]
                loop {
                    #[cfg(straw_verus)]
                    proof! { if remaining.remaining().len() > 0 { crate::catalog_state::pop_set(remaining.remaining()); } }
                    let Some(parent) = remaining.next() else {
                        break;
                    };
                    if let Err(error) = effects.sync_parent(parent) {
                        return Err(Failure::Effect(error));
                    }
                }
                #[cfg(straw_verus)]
                proof! {
                    assert(remaining.remaining().unref().to_set() =~= Set::<E::Parent>::empty());
                    assert forall|j: nat| indices(effects@).contains(j)
                        implies effects@.durable_absent.contains(j) by {
                        assert(!remaining.remaining().unref().to_set().contains(effects@.parents[j as int]));
                    }
                    assert(effects@.durable_absent =~= indices(effects@));
                }
                if !protocol::delete_batch_finished(&pending, true) {
                    return Err(Failure::Protocol);
                }
            }
            ReclaimEffect::RecordReceipt => {
                if let Err(error) = effects.receipt() {
                    return Err(Failure::Effect(error));
                }
            }
        }
        state = protocol::reclaim_advance(state, true);
    }
}
