//! Conditional completion across failed/restarted production controller Views.
//! The caller supplies fair retries; actual I/O and recovery refinement is not proved.
use vstd::prelude::*;
use crate::protocol::{ProgressView, initial, advance};
use crate::environment::{control_stable_from, control_eventually_stable,
    control_weak_fair, control_fair_at};

verus! {
    pub enum RetryEvent { Idle, Io { success: bool }, Retry, Crash }
    pub open spec fn io_enabled(s: ProgressView, limit: nat) -> bool {
        !s.failed && s.completed < limit
    }
    pub open spec fn retry_enabled(s: ProgressView) -> bool { s.failed }
    pub open spec fn valid(s: ProgressView, limit: nat) -> bool {
        s.completed <= limit && (s.completed == limit ==> !s.failed)
    }
    pub open spec fn step(s: ProgressView, e: RetryEvent, n: ProgressView, limit: nat) -> bool {
        match e {
            RetryEvent::Idle => n == s,
            RetryEvent::Io { success } => io_enabled(s, limit) && n == advance(s, success, limit),
            RetryEvent::Retry => retry_enabled(s) && n == initial(),
            RetryEvent::Crash => s.completed < limit && n == ProgressView { completed: 0, failed: true },
        }
    }
    pub open spec fn history(t: spec_fn(nat) -> ProgressView, e: spec_fn(nat) -> RetryEvent, limit: nat) -> bool {
        t(0) == initial() && forall|i: nat| #![trigger t(i)] step(t(i), e(i), t(i + 1), limit)
    }
    pub open spec fn rank(s: ProgressView, limit: nat) -> nat {
        if s.failed { limit + 1 } else { (limit - s.completed) as nat }
    }
    pub proof fn history_valid(t: spec_fn(nat) -> ProgressView, e: spec_fn(nat) -> RetryEvent, limit: nat, at: nat)
        requires history(t, e, limit), ensures valid(t(at), limit), decreases at,
    {
        if at > 0 { history_valid(t, e, limit, (at - 1) as nat); }
    }
    pub proof fn stable_step(s: ProgressView, e: RetryEvent, n: ProgressView, limit: nat)
        requires valid(s, limit), step(s, e, n, limit), !matches!(e, RetryEvent::Crash),
            matches!(e, RetryEvent::Io { .. }) ==> e->success,
        ensures rank(n, limit) <= rank(s, limit), !matches!(e, RetryEvent::Idle) ==> rank(n, limit) < rank(s, limit),
    {}
    pub proof fn stable_response_at(e: spec_fn(nat) -> RetryEvent, stable: nat, at: nat)
        requires control_stable_from(e, stable), at >= stable,
        ensures !matches!(e(at), RetryEvent::Crash), matches!(e(at), RetryEvent::Io { .. }) ==> e(at)->success,
    {}
    pub proof fn fair_retry_event(t: spec_fn(nat) -> ProgressView, e: spec_fn(nat) -> RetryEvent, limit: nat, start: nat)
        requires control_fair_at(t, e, limit, start),
            forall|i: nat| #![trigger t(i)] i >= start ==> retry_enabled(t(i)),
        ensures exists|i: nat| #![trigger e(i)] i >= start && matches!(e(i), RetryEvent::Retry),
    {}
    pub proof fn fair_io_event(t: spec_fn(nat) -> ProgressView, e: spec_fn(nat) -> RetryEvent, limit: nat, start: nat)
        requires control_fair_at(t, e, limit, start),
            forall|i: nat| #![trigger t(i)] i >= start ==> io_enabled(t(i), limit),
        ensures exists|i: nat| #![trigger e(i)] i >= start && matches!(e(i), RetryEvent::Io { .. }),
    {}
    pub proof fn rank_nonincreasing(t: spec_fn(nat) -> ProgressView, e: spec_fn(nat) -> RetryEvent,
        limit: nat, stable: nat, start: nat, end: nat)
        requires history(t, e, limit), control_stable_from(e, stable), stable <= start <= end,
        ensures rank(t(end), limit) <= rank(t(start), limit), decreases end - start,
    {
        if end > start {
            rank_nonincreasing(t, e, limit, stable, start, (end - 1) as nat);
            history_valid(t, e, limit, (end - 1) as nat);
            stable_response_at(e, stable, (end - 1) as nat);
            stable_step(t((end - 1) as nat), e((end - 1) as nat), t(end), limit);
        }
    }
    pub proof fn eventually_lower(t: spec_fn(nat) -> ProgressView, e: spec_fn(nat) -> RetryEvent,
        limit: nat, stable: nat, start: nat)
        requires history(t, e, limit), control_stable_from(e, stable), control_weak_fair(t, e, limit),
            stable <= start, rank(t(start), limit) > 0,
        ensures exists|end: nat| #![trigger t(end)] end > start && rank(t(end), limit) < rank(t(start), limit),
    {
        if !(exists|end: nat| #![trigger t(end)] end > start && rank(t(end), limit) < rank(t(start), limit)) {
            assert forall|i: nat| #![trigger t(i)] i >= start implies rank(t(i), limit) == rank(t(start), limit) by {
                rank_nonincreasing(t, e, limit, stable, start, i);
            }
            history_valid(t, e, limit, start);
            assert(control_fair_at(t, e, limit, start));
            if retry_enabled(t(start)) {
                assert forall|i: nat| #![trigger t(i)] i >= start implies retry_enabled(t(i)) by {
                    history_valid(t, e, limit, i);
                }
                fair_retry_event(t, e, limit, start);
                let at = choose|i: nat| #![trigger e(i)] i >= start && matches!(e(i), RetryEvent::Retry);
                history_valid(t, e, limit, at);
                stable_response_at(e, stable, at);
                stable_step(t(at), e(at), t(at + 1), limit);
            } else {
                assert forall|i: nat| #![trigger t(i)] i >= start implies io_enabled(t(i), limit) by {
                    history_valid(t, e, limit, i);
                }
                fair_io_event(t, e, limit, start);
                let at = choose|i: nat| #![trigger e(i)] i >= start && matches!(e(i), RetryEvent::Io { .. });
                history_valid(t, e, limit, at);
                stable_response_at(e, stable, at);
                stable_step(t(at), e(at), t(at + 1), limit);
            }
            assert(false);
        }
    }
    pub proof fn eventually_zero(t: spec_fn(nat) -> ProgressView, e: spec_fn(nat) -> RetryEvent,
        limit: nat, stable: nat, start: nat)
        requires history(t, e, limit), control_stable_from(e, stable), control_weak_fair(t, e, limit), stable <= start,
        ensures exists|end: nat| #![trigger t(end)] end >= start && rank(t(end), limit) == 0,
        decreases rank(t(start), limit),
    {
        if rank(t(start), limit) > 0 {
            eventually_lower(t, e, limit, stable, start);
            let next = choose|end: nat| #![trigger t(end)] end > start && rank(t(end), limit) < rank(t(start), limit);
            eventually_zero(t, e, limit, stable, next);
            let end = choose|end: nat| #![trigger t(end)] end >= next && rank(t(end), limit) == 0;
            assert(end >= start);
        } else { assert(start >= start); }
    }
    // This theorem derives completion without a direct eventual-stage-success
    // premise. It allows any finite number of errors and
    // crashes before stabilization, with no elapsed-time bound or retry quota.
    pub proof fn eventually_finishes_from_environment(t: spec_fn(nat) -> ProgressView,
        e: spec_fn(nat) -> RetryEvent, limit: nat)
        requires history(t, e, limit), control_eventually_stable(e), control_weak_fair(t, e, limit),
        ensures exists|end: nat| #![trigger t(end)] !t(end).failed && t(end).completed == limit,
    {
        let stable = choose|at: nat| control_stable_from(e, at);
        eventually_zero(t, e, limit, stable, stable);
        let end = choose|end: nat| #![trigger t(end)] end >= stable && rank(t(end), limit) == 0;
        history_valid(t, e, limit, end);
    }
    // A four-stage successful history following an error and caller retry.
    pub open spec fn example_state(i: nat) -> ProgressView {
        ProgressView { completed: if i < 3 { 0 } else if i < 6 { (i - 2) as nat } else { 4 }, failed: i == 1 }
    }
    pub open spec fn example_event(i: nat) -> RetryEvent {
        if i == 0 { RetryEvent::Io { success: false } }
        else if i == 1 { RetryEvent::Retry }
        else if i < 6 { RetryEvent::Io { success: true } }
        else { RetryEvent::Idle }
    }
    pub proof fn retry_example_exists()
        ensures history(|i: nat| example_state(i), |i: nat| example_event(i), 4),
            control_eventually_stable(|i: nat| example_event(i)),
            control_weak_fair(|i: nat| example_state(i), |i: nat| example_event(i), 4),
            example_state(6).completed == 4,
    {
        assert(control_stable_from(|i: nat| example_event(i), 1));
        let t = |i: nat| example_state(i);
        let e = |i: nat| example_event(i);
        assert forall|start: nat| control_fair_at(t, e, 4, start) by {
            assert(!io_enabled(t(start + 6), 4));
            assert(!retry_enabled(t(start + 6)));
        }
    }
    pub open spec fn abandoned_state(i: nat) -> ProgressView {
        ProgressView { completed: 0, failed: i > 0 }
    }
    pub open spec fn abandoned_event(i: nat) -> RetryEvent {
        if i == 0 { RetryEvent::Io { success: false } } else { RetryEvent::Idle }
    }
    pub proof fn caller_retry_is_necessary()
        ensures history(|i: nat| abandoned_state(i), |i: nat| abandoned_event(i), 4),
            control_eventually_stable(|i: nat| abandoned_event(i)),
            !control_weak_fair(|i: nat| abandoned_state(i), |i: nat| abandoned_event(i), 4),
            forall|i: nat| #[trigger] abandoned_state(i).completed < 4,
    {
        assert(control_stable_from(|i: nat| abandoned_event(i), 1));
        assert forall|i: nat| #![trigger abandoned_state(i)] i >= 1 implies retry_enabled(abandoned_state(i)) by {}
        assert(!control_fair_at(|i: nat| abandoned_state(i), |i: nat| abandoned_event(i), 4, 1));
    }
}
