//! Unbounded composition of queue receipts, retained roots, handoff and GC.
//! Durable effects are the abstraction boundary, not a proved native adapter.
use vstd::prelude::*;

verus! {
    pub struct Key { pub queue: nat, pub request: nat }
    pub struct Receipt { pub digest: nat, pub stage: nat }
    pub enum Reply { New, Cached(Receipt), Conflict }
    pub struct State {
        pub recipes: Map<nat, Set<nat>>,
        pub queues: Map<nat, Set<nat>>,
        pub holders: Map<nat, Set<nat>>,
        pub stages: Map<nat, nat>,
        pub generation: Map<nat, nat>,
        pub accepted: Map<Key, Receipt>,
        pub cached: Map<Key, Receipt>,
        pub count: Map<Key, nat>,
        pub retired: Set<Key>,
        pub consumed: Set<Key>,
        pub ready: Set<nat>,
        pub sealed: Set<nat>,
        pub present: Set<nat>,
        pub durable_names: Set<nat>,
        pub dead: Set<nat>,
        pub reclaimed: Set<nat>,
    }
    pub enum Action {
        Retain { queue: nat, stage: nat },
        Accept { key: Key, digest: nat, stage: nat, applied: bool },
        Retire(Key), Prune { queue: nat, keep: Set<nat> },
        Hold { owner: nat, stage: nat }, Release(nat), Stage(nat), Consume(Key),
        Mark(nat), Unlink(nat), DirectorySync(nat), Receipt(nat),
        CrashStorage(Set<nat>), CrashQueue(nat), RecoverQueue(nat), Idle,
    }
    pub open spec fn owned(s: State, q: nat) -> Set<nat> {
        if s.queues.contains_key(q) { s.queues[q] } else { Set::empty() }
    }
    pub open spec fn protected(s: State, p: nat) -> bool {
        (exists|q: nat| s.queues.contains_key(q) && s.queues[q].contains(p))
            || (exists|h: nat| s.holders.contains_key(h) && s.holders[h].contains(p))
            || (exists|t: nat| s.stages.contains_key(t) && s.recipes[t].contains(p))
    }
    pub open spec fn available(s: State, roots: Set<nat>) -> bool {
        roots.subset_of(s.present) && roots.disjoint(s.dead)
    }
    pub open spec fn active(s: State, k: Key) -> bool {
        s.accepted.contains_key(k) && !s.retired.contains(k)
    }
    pub open spec fn keeps(s: State, q: nat, roots: Set<nat>) -> bool {
        forall|k: Key| active(s, k) && k.queue == q ==> s.recipes[s.accepted[k].stage].subset_of(roots)
    }
    pub open spec fn cache_valid(s: State) -> bool {
        s.cached.dom().subset_of(s.accepted.dom())
            && (forall|k: Key| s.cached.contains_key(k) ==> s.cached[k] == s.accepted[k])
            && (forall|k: Key| s.accepted.contains_key(k) && s.ready.contains(k.queue) ==> s.cached.contains_key(k))
    }
    #[verifier::opaque]
    pub open spec fn valid(s: State) -> bool {
        s.generation.dom() == s.recipes.dom() && s.stages.dom().subset_of(s.recipes.dom())
            && (forall|t: nat| s.stages.contains_key(t) ==> s.stages[t] == s.generation[t])
            && (forall|k: Key| s.accepted.contains_key(k) ==> s.recipes.contains_key(s.accepted[k].stage))
            && s.retired.subset_of(s.accepted.dom()) && s.consumed.subset_of(s.accepted.dom())
            && s.count.dom() == s.accepted.dom()
            && (forall|k: Key| s.count.contains_key(k) ==> s.count[k] == 1)
            && cache_valid(s)
            && (forall|k: Key| active(s, k) ==> s.recipes[s.accepted[k].stage].subset_of(owned(s, k.queue)))
            && (forall|p: nat| protected(s, p) ==> s.present.contains(p) && !s.dead.contains(p))
            && s.present.subset_of(s.durable_names)
            && s.dead.subset_of(s.sealed) && s.reclaimed.subset_of(s.dead) && s.reclaimed.disjoint(s.durable_names)
    }
    pub open spec fn initial(recipes: Map<nat, Set<nat>>, present: Set<nat>, sealed: Set<nat>) -> State {
        State { recipes, queues: Map::empty(), holders: Map::empty(), stages: Map::empty(),
            generation: Map::new(recipes.dom(), |t: nat| 0),
            accepted: Map::empty(), cached: Map::empty(), count: Map::empty(),
            retired: Set::empty(), consumed: Set::empty(), ready: Set::empty(),
            sealed, present, durable_names: present, dead: Set::empty(), reclaimed: Set::empty() }
    }
    #[verifier::opaque]
    pub open spec fn enabled(s: State, a: Action) -> bool {
        match a {
            Action::Retain { queue, stage } => s.ready.contains(queue) && s.recipes.contains_key(stage) && available(s, s.recipes[stage]),
            Action::Accept { key, digest, stage, applied } => s.ready.contains(key.queue)
                && !s.accepted.contains_key(key) && s.recipes.contains_key(stage)
                && s.recipes[stage].subset_of(owned(s, key.queue)),
            Action::Retire(key) => s.ready.contains(key.queue) && s.accepted.contains_key(key),
            Action::Prune { queue, keep } => s.ready.contains(queue) && keeps(s, queue, keep),
            Action::Hold { owner, stage } => s.recipes.contains_key(stage) && available(s, s.recipes[stage]),
            Action::Release(_) => true,
            Action::Stage(stage) => s.recipes.contains_key(stage) && available(s, s.recipes[stage]),
            Action::Consume(key) => s.accepted.contains_key(key)
                && (s.consumed.contains(key) || active(s, key)),
            Action::Mark(p) => s.sealed.contains(p) && !protected(s, p),
            Action::Unlink(p) => s.dead.contains(p),
            Action::DirectorySync(p) => !s.present.contains(p),
            Action::Receipt(p) => s.dead.contains(p) && !s.durable_names.contains(p),
            Action::CrashStorage(restored) => crate::environment::composition_restored_names(s.present, s.durable_names, restored),
            Action::CrashQueue(_) | Action::RecoverQueue(_) | Action::Idle => true,
        }
    }
    pub open spec fn recover_cache(s: State, q: nat) -> Map<Key, Receipt> {
        Map::new(s.cached.dom().union(s.accepted.dom().filter(|k: Key| k.queue == q)),
            |k: Key| if k.queue == q { s.accepted[k] } else { s.cached[k] })
    }
    #[verifier::opaque]
    pub open spec fn effect(s: State, a: Action) -> State {
        match a {
            Action::Retain { queue, stage } => State { queues: s.queues.insert(queue, owned(s, queue).union(s.recipes[stage])), ..s },
            Action::Accept { key, digest, stage, applied } => {
                let receipt = Receipt { digest, stage };
                State { accepted: s.accepted.insert(key, receipt),
                    cached: if applied { s.cached.insert(key, receipt) } else { s.cached },
                    ready: if applied { s.ready } else { s.ready.remove(key.queue) },
                    count: s.count.insert(key, if s.count.contains_key(key) { s.count[key] + 1 } else { 1 }), ..s }
            },
            Action::Retire(key) => State { retired: s.retired.insert(key), ..s },
            Action::Prune { queue, keep } => State { queues: s.queues.insert(queue, owned(s, queue).intersect(keep)), ..s },
            Action::Hold { owner, stage } => State { holders: s.holders.insert(owner, s.recipes[stage]), ..s },
            Action::Release(owner) => State { holders: s.holders.remove(owner), ..s },
            Action::Stage(stage) => State { stages: s.stages.insert(stage, s.generation[stage] + 1),
                generation: s.generation.insert(stage, s.generation[stage] + 1), ..s },
            Action::Consume(key) => if s.consumed.contains(key) { s } else {
                State { consumed: s.consumed.insert(key), stages: s.stages.remove(s.accepted[key].stage), ..s }
            },
            Action::Mark(p) => State { dead: s.dead.insert(p), ..s },
            Action::Unlink(p) => State { present: s.present.remove(p), ..s },
            Action::DirectorySync(p) => State { durable_names: crate::environment::composition_synced_names(s.durable_names, p), ..s },
            Action::Receipt(p) => State { reclaimed: s.reclaimed.insert(p), ..s },
            Action::CrashStorage(restored) => State { present: restored, ..s },
            Action::CrashQueue(q) => State { ready: s.ready.remove(q), ..s },
            Action::RecoverQueue(q) => State { ready: s.ready.insert(q), cached: recover_cache(s, q), ..s },
            Action::Idle => s,
        }
    }
    pub open spec fn next(s: State, a: Action, n: State) -> bool { enabled(s, a) && n == effect(s, a) }
    pub open spec fn retry(s: State, k: Key, digest: nat) -> Reply {
        if !s.cached.contains_key(k) { Reply::New }
        else if s.cached[k].digest == digest { Reply::Cached(s.cached[k]) } else { Reply::Conflict }
    }

    pub proof fn initial_safe(recipes: Map<nat, Set<nat>>, present: Set<nat>, sealed: Set<nat>)
        ensures valid(initial(recipes, present, sealed)),
    { reveal(valid); reveal(enabled); reveal(effect);}

    pub proof fn protection_step(s: State, a: Action)
        requires valid(s), enabled(s, a),
        ensures forall|p: nat| protected(effect(s, a), p)
            ==> effect(s, a).present.contains(p) && !effect(s, a).dead.contains(p),
    { reveal(valid); reveal(enabled); reveal(effect);
        let n = effect(s, a);
        assert forall|p: nat| protected(n, p) implies n.present.contains(p) && !n.dead.contains(p) by {
            match a {
                Action::Retain { queue, stage } => {
                    if !protected(s, p) {
                        assert(s.recipes[stage].contains(p)) by {
                            if exists|q: nat| n.queues.contains_key(q) && n.queues[q].contains(p) {
                                let q = choose|q: nat| n.queues.contains_key(q) && n.queues[q].contains(p);
                                if q != queue { assert(protected(s, p)); }
                                else if owned(s, queue).contains(p) { assert(protected(s, p)); }
                            } else if exists|h: nat| n.holders.contains_key(h) && n.holders[h].contains(p) {
                                let h = choose|h: nat| n.holders.contains_key(h) && n.holders[h].contains(p);
                                assert(protected(s, p));
                            } else {
                                let t = choose|t: nat| n.stages.contains_key(t) && n.recipes[t].contains(p);
                                assert(protected(s, p));
                            }
                        }
                    }
                },
                Action::Hold { owner, stage } => {
                    if !protected(s, p) {
                        let h = choose|h: nat| n.holders.contains_key(h) && n.holders[h].contains(p);
                        if h != owner { assert(protected(s, p)); }
                        assert(s.recipes[stage].contains(p));
                    }
                },
                Action::Stage(stage) => {
                    if !protected(s, p) {
                        let t = choose|t: nat| n.stages.contains_key(t) && n.recipes[t].contains(p);
                        if t != stage { assert(protected(s, p)); }
                        assert(s.recipes[stage].contains(p));
                    }
                },
                _ => {
                    if exists|q: nat| n.queues.contains_key(q) && n.queues[q].contains(p) {
                        let q = choose|q: nat| n.queues.contains_key(q) && n.queues[q].contains(p);
                        assert(s.queues.contains_key(q) && s.queues[q].contains(p));
                    } else if exists|h: nat| n.holders.contains_key(h) && n.holders[h].contains(p) {
                        let h = choose|h: nat| n.holders.contains_key(h) && n.holders[h].contains(p);
                        assert(s.holders.contains_key(h) && s.holders[h].contains(p));
                    } else {
                        let t = choose|t: nat| n.stages.contains_key(t) && n.recipes[t].contains(p);
                        assert(s.stages.contains_key(t) && s.recipes[t].contains(p));
                    }
                    assert(protected(s, p));
                },
            }
        }
    }
    pub proof fn queue_step(s: State, a: Action)
        requires valid(s), enabled(s, a),
        ensures forall|k: Key| active(effect(s, a), k) ==>
            effect(s, a).recipes[effect(s, a).accepted[k].stage].subset_of(owned(effect(s, a), k.queue)),
    { reveal(valid); reveal(enabled); reveal(effect);
        let n = effect(s, a);
        assert forall|k: Key| active(n, k) implies n.recipes[n.accepted[k].stage].subset_of(owned(n, k.queue)) by {
            match a {
                Action::Accept { key, .. } => { if k != key { assert(active(s, k)); } },
                Action::Prune { queue, keep } => {
                    assert(active(s, k));
                    if k.queue == queue {
                        assert(s.recipes[s.accepted[k].stage].subset_of(keep));
                        assert(s.recipes[s.accepted[k].stage].subset_of(owned(s, queue).intersect(keep)));
                    }
                },
                _ => { assert(active(s, k)); },
            }
        }
    }
    pub proof fn cache_step(s: State, a: Action)
        requires valid(s), enabled(s, a),
        ensures cache_valid(effect(s, a)),
    { reveal(valid); reveal(enabled); reveal(effect);
        let n = effect(s, a);
        assert forall|k: Key| n.cached.contains_key(k) implies n.accepted.contains_key(k) && n.cached[k] == n.accepted[k] by {};
        assert forall|k: Key| n.accepted.contains_key(k) && n.ready.contains(k.queue) implies n.cached.contains_key(k) by {};
    }
    pub proof fn step_safe(s: State, a: Action)
        requires valid(s), enabled(s, a),
        ensures valid(effect(s, a)),
    { reveal(valid); reveal(enabled); reveal(effect);
        protection_step(s, a); queue_step(s, a); cache_step(s, a);
        let n = effect(s, a);
        assert(n.generation.dom() == n.recipes.dom());
        assert(n.stages.dom().subset_of(n.recipes.dom()));
        assert(n.retired.subset_of(n.accepted.dom()));
        assert(n.consumed.subset_of(n.accepted.dom()));
        assert(n.count.dom() == n.accepted.dom());
        assert(n.present.subset_of(n.durable_names));
        assert(n.dead.subset_of(n.sealed));
        assert(n.reclaimed.subset_of(n.dead));
        assert(n.reclaimed.disjoint(n.durable_names));
        assert forall|k: Key| n.accepted.contains_key(k) implies n.recipes.contains_key(n.accepted[k].stage) by {};
        assert forall|k: Key| n.count.contains_key(k) implies n.count[k] == 1 by {};
        assert forall|t: nat| n.stages.contains_key(t) implies n.stages[t] == n.generation[t] by {};
    }
    pub proof fn step_preserves_receipt(s: State, a: Action, k: Key)
        requires valid(s), enabled(s, a), s.accepted.contains_key(k),
        ensures effect(s, a).accepted.contains_key(k) && effect(s, a).accepted[k] == s.accepted[k],
    { reveal(valid); reveal(enabled); reveal(effect);}
    pub proof fn step_preserves_fences(s: State, a: Action)
        requires valid(s), enabled(s, a),
        ensures s.dead.subset_of(effect(s, a).dead), s.consumed.subset_of(effect(s, a).consumed),
            s.retired.subset_of(effect(s, a).retired), s.reclaimed.subset_of(effect(s, a).reclaimed),
    { reveal(valid); reveal(enabled); reveal(effect);}
    pub proof fn live_request_is_protected(s: State, k: Key, p: nat)
        requires valid(s), active(s, k), s.recipes[s.accepted[k].stage].contains(p),
        ensures s.present.contains(p), !s.dead.contains(p), !s.reclaimed.contains(p),
    { reveal(valid); reveal(enabled); reveal(effect); assert(owned(s, k.queue).contains(p)); assert(protected(s, p)); }
    pub proof fn cached_retry_after_gc(s: State, k: Key, digest: nat)
        requires valid(s), s.ready.contains(k.queue), s.accepted.contains_key(k),
        ensures retry(s, k, digest) == if digest == s.accepted[k].digest { Reply::Cached(s.accepted[k]) } else { Reply::Conflict },
    { reveal(valid); reveal(enabled); reveal(effect);}
    pub proof fn retained_closure_is_complete(s: State, q: nat, stage: nat)
        requires enabled(s, Action::Retain { queue: q, stage }),
        ensures s.recipes[stage].subset_of(owned(effect(s, Action::Retain { queue: q, stage }), q)),
    { reveal(enabled); reveal(effect); }
    pub proof fn old_handoff_preserves_restage(s: State, k: Key)
        requires s.consumed.contains(k),
        ensures effect(s, Action::Consume(k)) == s,
    { reveal(valid); reveal(enabled); reveal(effect);}
    pub open spec fn history(trace: spec_fn(nat) -> State, actions: spec_fn(nat) -> Action) -> bool {
        valid(trace(0)) && forall|i: nat| #![trigger trace(i)] next(trace(i), actions(i), trace(i + 1))
    }
    pub proof fn history_safe(trace: spec_fn(nat) -> State, actions: spec_fn(nat) -> Action, at: nat)
        requires history(trace, actions),
        ensures valid(trace(at)), decreases at,
    {
        if at > 0 {
            history_safe(trace, actions, (at - 1) as nat);
            step_safe(trace((at - 1) as nat), actions((at - 1) as nat));
        }
    }
    pub proof fn history_preserves_receipt(trace: spec_fn(nat) -> State, actions: spec_fn(nat) -> Action, start: nat, end: nat, k: Key)
        requires history(trace, actions), start <= end, trace(start).accepted.contains_key(k),
        ensures trace(end).accepted.contains_key(k) && trace(end).accepted[k] == trace(start).accepted[k],
        decreases end - start,
    {
        if start < end {
            history_preserves_receipt(trace, actions, start, (end - 1) as nat, k);
            history_safe(trace, actions, (end - 1) as nat);
            step_preserves_receipt(trace((end - 1) as nat), actions((end - 1) as nat), k);
        }
    }
    pub proof fn take_step(s: State, a: Action) -> (n: State)
        requires valid(s), enabled(s, a),
        ensures n == effect(s, a), valid(n),
    { step_safe(s, a); effect(s, a) }

    // A concrete reachable history prevents an inconsistent/vacuous model:
    // two queue-scoped equal IDs, lost apply, recovery, restaging, old cleanup,
    // and one queue's retirement while the other still needs both packs.
    pub proof fn shared_restage_example() -> (s: State)
        ensures valid(s),
            s.accepted.contains_key(Key { queue: 1, request: 11 }),
            s.accepted.contains_key(Key { queue: 2, request: 11 }),
            s.retired.contains(Key { queue: 1, request: 11 }),
            active(s, Key { queue: 2, request: 11 }),
            s.stages.contains_key(7) && s.stages[7] == 2,
            s.consumed.contains(Key { queue: 1, request: 11 }),
            s.present.contains(1) && s.present.contains(2),
            owned(s, 1) == Set::<nat>::empty(),
            s.cached[Key { queue: 1, request: 11 }].digest == 3,
            s.cached[Key { queue: 2, request: 11 }].digest == 4,
    {
        reveal(enabled); reveal(effect);
        let roots = Set::<nat>::empty().insert(1).insert(2);
        let recipes = Map::<nat, Set<nat>>::empty().insert(7, roots);
        initial_safe(recipes, roots, roots);
        let s0 = initial(recipes, roots, roots);
        let s1 = take_step(s0, Action::RecoverQueue(1));
        let s2 = take_step(s1, Action::RecoverQueue(2));
        let s3 = take_step(s2, Action::Stage(7));
        let s4 = take_step(s3, Action::Retain { queue: 1, stage: 7 });
        let s5 = take_step(s4, Action::Retain { queue: 2, stage: 7 });
        let k1 = Key { queue: 1, request: 11 };
        let k2 = Key { queue: 2, request: 11 };
        let s6 = take_step(s5, Action::Accept { key: k1, digest: 3, stage: 7, applied: false });
        let s7 = take_step(s6, Action::Accept { key: k2, digest: 4, stage: 7, applied: true });
        let s8 = take_step(s7, Action::RecoverQueue(1));
        let s9 = take_step(s8, Action::Consume(k1));
        let s10 = take_step(s9, Action::Stage(7));
        let s11 = take_step(s10, Action::Consume(k1));
        assert(s11 == s10);
        let s12 = take_step(s11, Action::Retire(k1));
        take_step(s12, Action::Prune { queue: 1, keep: Set::empty() })
    }

    // A caller must establish this root correspondence from its actual locked
    // catalog snapshot; it is not granted by the upper protocol model.
    pub proof fn active_request_excludes_actual_candidate(s: State, catalog: crate::catalog_state::State,
        key: Key, pack: nat, path: String)
        requires valid(s), active(s, key), s.recipes[s.accepted[key].stage].contains(pack),
            crate::catalog_state::rooted(catalog, path) == protected(s, pack),
        ensures !crate::catalog_state::eligible(catalog, path),
    {
        reveal(valid);
        assert(owned(s, key.queue).contains(pack));
        assert(protected(s, pack));
    }

}
