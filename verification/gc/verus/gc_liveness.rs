//! One permanently unowned, durably tombstoned pack, across deletion attempts.
//! Uses the production controller's View; syscall/recovery adapters are modeled.
use vstd::prelude::*;
use crate::protocol::{ProgressView, initial, advance};
use crate::environment::{gc_stable_from, gc_eventually_stable, gc_weak_fair, gc_fair_at,
    gc_io_response, gc_crash_response};

verus! {
    pub struct GcState {
        pub controller: ProgressView,
        pub present: bool,
        pub durable_present: bool,
    }
    pub enum GcEvent {
        Idle,
        Io { success: bool, after_present: bool, after_durable: bool },
        Retry,
        Crash { restored: bool },
    }
    pub open spec fn io_enabled(s: GcState) -> bool {
        !s.controller.failed && s.controller.completed < 2
    }
    pub open spec fn retry_enabled(s: GcState) -> bool { s.controller.failed }
    pub open spec fn valid(s: GcState) -> bool {
        s.controller.completed <= 2
        && (s.controller.completed == 2 ==> !s.controller.failed && !s.durable_present)
        && (!s.controller.failed && s.controller.completed == 1 ==> !s.present)
    }
    pub open spec fn init(s: GcState) -> bool { s.controller == initial() }
    pub open spec fn step(s: GcState, e: GcEvent, n: GcState) -> bool {
        match e {
            GcEvent::Idle => n == s,
            GcEvent::Io { success, after_present, after_durable } => {
                io_enabled(s) && n.controller == advance(s.controller, success, 2)
                && gc_io_response(s, success, after_present, after_durable, n)
            },
            GcEvent::Retry => retry_enabled(s) && n.controller == initial()
                && n.present == s.present && n.durable_present == s.durable_present,
            GcEvent::Crash { restored } => s.controller.completed < 2
                && n.controller == initial() && gc_crash_response(s, restored, n),
        }
    }
    pub open spec fn history(t: spec_fn(nat) -> GcState, e: spec_fn(nat) -> GcEvent) -> bool {
        init(t(0)) && forall|i: nat| #![trigger t(i)] step(t(i), e(i), t(i + 1))
    }
    pub open spec fn rank(s: GcState) -> nat {
        if s.controller.failed { 3 } else { (2 - s.controller.completed) as nat }
    }
    pub proof fn history_valid(t: spec_fn(nat) -> GcState, e: spec_fn(nat) -> GcEvent, at: nat)
        requires history(t, e), ensures valid(t(at)), decreases at,
    {
        if at > 0 { history_valid(t, e, (at - 1) as nat); }
    }
    pub proof fn stable_step(s: GcState, e: GcEvent, n: GcState)
        requires valid(s), step(s, e, n), !matches!(e, GcEvent::Crash { .. }), matches!(e, GcEvent::Io { .. }) ==> e->success,
        ensures rank(n) <= rank(s), !matches!(e, GcEvent::Idle) ==> rank(n) < rank(s),
    {}
    pub proof fn stable_response_at(e: spec_fn(nat) -> GcEvent, stable: nat, at: nat)
        requires gc_stable_from(e, stable), at >= stable,
        ensures !matches!(e(at), GcEvent::Crash { .. }),
            matches!(e(at), GcEvent::Io { .. }) ==> e(at)->success,
    {}
    pub proof fn fair_retry_event(t: spec_fn(nat) -> GcState, e: spec_fn(nat) -> GcEvent, start: nat)
        requires gc_fair_at(t, e, start),
            forall|i: nat| #![trigger t(i)] i >= start ==> retry_enabled(t(i)),
        ensures exists|i: nat| #![trigger e(i)] i >= start && matches!(e(i), GcEvent::Retry),
    {}
    pub proof fn rank_nonincreasing(t: spec_fn(nat) -> GcState, e: spec_fn(nat) -> GcEvent,
        stable: nat, start: nat, end: nat)
        requires history(t, e), gc_stable_from(e, stable), stable <= start <= end,
        ensures rank(t(end)) <= rank(t(start)), decreases end - start,
    {
        if end > start {
            rank_nonincreasing(t, e, stable, start, (end - 1) as nat);
            history_valid(t, e, (end - 1) as nat);
            stable_response_at(e, stable, (end - 1) as nat);
            stable_step(t((end - 1) as nat), e((end - 1) as nat), t(end));
        }
    }
    pub proof fn eventually_lower(t: spec_fn(nat) -> GcState, e: spec_fn(nat) -> GcEvent,
        stable: nat, start: nat)
        requires history(t, e), gc_stable_from(e, stable), gc_weak_fair(t, e),
            stable <= start, rank(t(start)) > 0,
        ensures exists|end: nat| #![trigger t(end)] end > start && rank(t(end)) < rank(t(start)),
    {
        if !(exists|end: nat| #![trigger t(end)] end > start && rank(t(end)) < rank(t(start))) {
            assert forall|i: nat| #![trigger t(i)] i >= start implies rank(t(i)) == rank(t(start)) by {
                rank_nonincreasing(t, e, stable, start, i);
            }
            history_valid(t, e, start);
            assert(gc_fair_at(t, e, start));
            if retry_enabled(t(start)) {
                assert forall|i: nat| #![trigger t(i)] i >= start implies retry_enabled(t(i)) by {
                    history_valid(t, e, i);
                }
                fair_retry_event(t, e, start);
                assert(exists|i: nat| #![trigger e(i)] i >= start && matches!(e(i), GcEvent::Retry));
                let at = choose|i: nat| #![trigger e(i)] i >= start && matches!(e(i), GcEvent::Retry);
                history_valid(t, e, at);
                stable_response_at(e, stable, at);
                stable_step(t(at), e(at), t(at + 1));
            } else {
                assert forall|i: nat| #![trigger t(i)] i >= start implies io_enabled(t(i)) by {
                    history_valid(t, e, i);
                }
                assert(exists|i: nat| #![trigger e(i)] i >= start && matches!(e(i), GcEvent::Io { .. }));
                let at = choose|i: nat| #![trigger e(i)] i >= start && matches!(e(i), GcEvent::Io { .. });
                history_valid(t, e, at);
                stable_response_at(e, stable, at);
                stable_step(t(at), e(at), t(at + 1));
            }
            assert(false);
        }
    }
    pub proof fn eventually_zero(t: spec_fn(nat) -> GcState, e: spec_fn(nat) -> GcEvent,
        stable: nat, start: nat)
        requires history(t, e), gc_stable_from(e, stable), gc_weak_fair(t, e), stable <= start,
        ensures exists|end: nat| #![trigger t(end)] end >= start && rank(t(end)) == 0,
        decreases rank(t(start)),
    {
        if rank(t(start)) > 0 {
            eventually_lower(t, e, stable, start);
            let next = choose|end: nat| #![trigger t(end)] end > start && rank(t(end)) < rank(t(start));
            eventually_zero(t, e, stable, next);
            let end = choose|end: nat| #![trigger t(end)] end >= next && rank(t(end)) == 0;
            assert(end >= start);
        } else { assert(start >= start); }
    }
    pub proof fn eventually_reclaims(t: spec_fn(nat) -> GcState, e: spec_fn(nat) -> GcEvent)
        requires history(t, e), gc_eventually_stable(e), gc_weak_fair(t, e),
        ensures exists|end: nat| #![trigger t(end)] !t(end).controller.failed && t(end).controller.completed == 2
            && !t(end).durable_present,
    {
        let stable = choose|at: nat| gc_stable_from(e, at);
        eventually_zero(t, e, stable, stable);
        let end = choose|end: nat| #![trigger t(end)] end >= stable && rank(t(end)) == 0;
        history_valid(t, e, end);
    }

    // A concrete infinite history witnesses that the progress premises are
    // jointly satisfiable: failed unlink with partial effect, retry, success.
    pub open spec fn example_state(i: nat) -> GcState {
        GcState {
            controller: if i == 1 { ProgressView { completed: 0, failed: true } }
                else if i < 3 { initial() }
                else { ProgressView { completed: if i == 3 { 1 } else { 2 }, failed: false } },
            present: i == 0,
            durable_present: i < 4,
        }
    }
    pub open spec fn example_event(i: nat) -> GcEvent {
        if i == 0 { GcEvent::Io { success: false, after_present: false, after_durable: true } }
        else if i == 1 { GcEvent::Retry }
        else if i < 4 { GcEvent::Io { success: true, after_present: false, after_durable: false } }
        else { GcEvent::Idle }
    }
    pub proof fn retry_example_exists()
        ensures history(|i: nat| example_state(i), |i: nat| example_event(i)), gc_eventually_stable(|i: nat| example_event(i)),
            gc_weak_fair(|i: nat| example_state(i), |i: nat| example_event(i)), !example_state(4).durable_present,
    {
        assert(gc_stable_from(|i: nat| example_event(i), 1));
        let t = |i: nat| example_state(i);
        let e = |i: nat| example_event(i);
        assert forall|start: nat| gc_fair_at(t, e, start) by {
            assert(!io_enabled(t(start + 4)));
            assert(!retry_enabled(t(start + 4)));
        }
    }

    // Without a caller retrying, storage may recover while the failed attempt
    // remains terminal forever. This is a proved counterexample, not a timeout.
    pub open spec fn abandoned_state(i: nat) -> GcState {
        GcState { controller: ProgressView { completed: 0, failed: i > 0 },
            present: i == 0, durable_present: true }
    }
    pub open spec fn abandoned_event(i: nat) -> GcEvent {
        if i == 0 { GcEvent::Io { success: false, after_present: false, after_durable: true } }
        else { GcEvent::Idle }
    }
    pub proof fn retry_premise_is_necessary()
        ensures history(|i: nat| abandoned_state(i), |i: nat| abandoned_event(i)), gc_eventually_stable(|i: nat| abandoned_event(i)),
            !gc_weak_fair(|i: nat| abandoned_state(i), |i: nat| abandoned_event(i)),
            forall|i: nat| #[trigger] abandoned_state(i).durable_present,
    {
        assert(gc_stable_from(|i: nat| abandoned_event(i), 1));
        assert forall|i: nat| #![trigger abandoned_state(i)] i >= 1 implies retry_enabled(abandoned_state(i)) by {}
        assert(!gc_fair_at(|i: nat| abandoned_state(i), |i: nat| abandoned_event(i), 1));
    }
}
