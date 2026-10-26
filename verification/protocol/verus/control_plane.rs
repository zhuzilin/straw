//! Unbounded model theorems for the executable controllers in rust/protocol.rs.
//! Catalog maps, I/O effects, locks and replay are modeled here, not Rust-proved.
use vstd::prelude::*;
use crate::protocol::*;
use crate::environment::serialized_catalog_history;

verus! {
    pub struct CatalogView {
        pub owners: Map<nat, Set<nat>>,
        pub sealed: Set<nat>,
        pub tombstones: Set<nat>,
        pub present: Set<nat>,
    }
    pub open spec fn live(c: CatalogView, p: nat) -> bool {
        exists|o: nat| c.owners.dom().contains(o) && c.owners[o].contains(p)
    }
    pub open spec fn safe(c: CatalogView) -> bool {
        forall|p: nat| live(c, p) ==> c.present.contains(p) && !c.tombstones.contains(p)
    }
    pub open spec fn candidates(c: CatalogView) -> Set<nat> {
        c.sealed.filter(|p: nat| !live(c, p) && !c.tombstones.contains(p))
    }
    pub open spec fn retain(c: CatalogView, owner: nat, closure: Set<nat>) -> CatalogView {
        CatalogView { owners: c.owners.insert(owner, closure), ..c }
    }
    pub open spec fn release(c: CatalogView, owner: nat) -> CatalogView {
        CatalogView { owners: c.owners.remove(owner), ..c }
    }
    pub open spec fn mark(c: CatalogView, selected: Set<nat>) -> CatalogView {
        CatalogView { tombstones: c.tombstones.union(selected), ..c }
    }
    pub open spec fn unlink(c: CatalogView, selected: Set<nat>) -> CatalogView {
        CatalogView { present: c.present.difference(selected), ..c }
    }
    pub open spec fn catalog_step(c: CatalogView, n: CatalogView) -> bool {
        // These are serialized *durable* catalog actions. The closed-over pack
        // set is an input; computing it from JSON/dependencies is an adapter.
        (exists|o: nat, roots: Set<nat>| roots.subset_of(c.present)
            && roots.disjoint(c.tombstones) && n == retain(c, o, roots))
        || (exists|o: nat| n == release(c, o))
        || (exists|selected: Set<nat>| selected.subset_of(candidates(c)) && n == mark(c, selected))
        || (exists|selected: Set<nat>| selected.subset_of(c.tombstones) && n == unlink(c, selected))
        || n == c
    }
    pub proof fn retain_safe(c: CatalogView, owner: nat, closure: Set<nat>)
        requires safe(c), closure.subset_of(c.present), closure.disjoint(c.tombstones),
        ensures safe(retain(c, owner, closure)),
    {
        let n = retain(c, owner, closure);
        assert forall|p: nat| live(n, p) implies n.present.contains(p) && !n.tombstones.contains(p) by {
            let o = choose|o: nat| n.owners.dom().contains(o) && n.owners[o].contains(p);
            if o != owner {
                assert(c.owners.dom().contains(o) && c.owners[o].contains(p));
                assert(live(c, p));
            }
        }
    }
    pub proof fn release_safe(c: CatalogView, owner: nat)
        requires safe(c), ensures safe(release(c, owner)),
    {
        let n = release(c, owner);
        assert forall|p: nat| live(n, p) implies n.present.contains(p) && !n.tombstones.contains(p) by {
            let o = choose|o: nat| n.owners.dom().contains(o) && n.owners[o].contains(p);
            assert(c.owners.dom().contains(o) && c.owners[o].contains(p));
            assert(live(c, p));
        }
    }
    pub proof fn mark_safe(c: CatalogView, selected: Set<nat>)
        requires safe(c), selected.subset_of(candidates(c)),
        ensures safe(mark(c, selected)),
            forall|p: nat| selected.contains(p) ==> c.sealed.contains(p) && !live(c, p),
    {
        let n = mark(c, selected);
        assert forall|p: nat| live(n, p) implies n.present.contains(p) && !n.tombstones.contains(p) by {
            assert(live(c, p));
        }
    }
    pub proof fn unlink_safe(c: CatalogView, selected: Set<nat>)
        requires safe(c), selected.subset_of(c.tombstones),
        ensures safe(unlink(c, selected)),
    {
        let n = unlink(c, selected);
        assert forall|p: nat| live(n, p) implies n.present.contains(p) && !n.tombstones.contains(p) by {
            assert(live(c, p));
        }
    }
    pub proof fn catalog_step_safe(c: CatalogView, n: CatalogView)
        requires safe(c), catalog_step(c, n), ensures safe(n),
    {
        if exists|o: nat, roots: Set<nat>| roots.subset_of(c.present) && roots.disjoint(c.tombstones) && n == retain(c, o, roots) {
            let (o, roots) = choose|o: nat, roots: Set<nat>| roots.subset_of(c.present) && roots.disjoint(c.tombstones) && n == retain(c, o, roots);
            retain_safe(c, o, roots);
        } else if exists|o: nat| n == release(c, o) {
            let o = choose|o: nat| n == release(c, o);
            release_safe(c, o);
        } else if exists|s: Set<nat>| s.subset_of(candidates(c)) && n == mark(c, s) {
            let s = choose|s: Set<nat>| s.subset_of(candidates(c)) && n == mark(c, s);
            mark_safe(c, s);
        } else if exists|s: Set<nat>| s.subset_of(c.tombstones) && n == unlink(c, s) {
            let s = choose|s: Set<nat>| s.subset_of(c.tombstones) && n == unlink(c, s);
            unlink_safe(c, s);
        }
    }
    pub proof fn catalog_history_safe(trace: spec_fn(nat) -> CatalogView, at: nat)
        requires safe(trace(0)), serialized_catalog_history(trace),
        ensures safe(trace(at)), decreases at,
    {
        if at > 0 {
            catalog_history_safe(trace, (at - 1) as nat);
            catalog_step_safe(trace((at - 1) as nat), trace(at));
        }
    }
    // Bridges the executable Boolean predicate to the set-based GC model.
    pub proof fn candidate_correspondence(c: CatalogView, p: nat, status: PackStatus)
        requires status.sealed == c.sealed.contains(p), status.owned == live(c, p),
            status.tombstoned == c.tombstones.contains(p),
        ensures eligible(status) == candidates(c).contains(p),
    {}

    pub open spec fn controller_step(s: ProgressView, n: ProgressView, limit: nat) -> bool {
        n == s || n == advance(s, true, limit) || n == advance(s, false, limit)
    }
    pub proof fn controller_history_bound(trace: spec_fn(nat) -> ProgressView, at: nat, limit: nat)
        requires trace(0) == initial(), forall|i: nat| #![trigger trace(i)] controller_step(trace(i), trace(i + 1), limit),
        ensures trace(at).completed <= limit, decreases at,
    {
        if at > 0 { controller_history_bound(trace, (at - 1) as nat, limit); }
    }
    pub proof fn errors_stop_effects(s: ProgressView, success: bool)
        requires s.failed,
        ensures advance(s, success, 4) == s, advance(s, success, 2) == s,
            commit_effect(s).is_none(), delete_effect(s, true).is_none(),
    {}
    pub proof fn commit_effect_order(s: ProgressView)
        ensures
            commit_effect(s) == Some(CommitEffect::AppendWal) ==> s.completed == 1,
            commit_effect(s) == Some(CommitEffect::Apply) ==> s.completed == 2,
            commit_effect(s) == Some(CommitEffect::ConsumeStaging) ==> s.completed == 3,
    {}
    pub proof fn delete_effect_order(s: ProgressView, tombstoned: bool)
        ensures
            delete_effect(s, tombstoned).is_some() ==> tombstoned,
            delete_effect(s, tombstoned) == Some(DeleteEffect::SyncParent) ==> s.completed == 1,
            !tombstoned ==> delete_effect(s, tombstoned).is_none(),
    {}

}
