//! Environment premises consumed by proofs; not derived filesystem theorems.
//! Review IDs and evidence in environment_contracts.json / ENVIRONMENT_MODELS.
use vstd::prelude::*;
use crate::retry_liveness::{RetryEvent, io_enabled as control_io_enabled, retry_enabled as control_retry_enabled};
use crate::protocol::ProgressView;
use crate::control_plane::{CatalogView, catalog_step};
use crate::gc_liveness::{GcState, GcEvent, io_enabled, retry_enabled};

verus! {
    // ENV-STRING-ORDER: the standard String ordering obeys vstd's ordered-key
    // contract. This premise is not derived by the publication holding proof.
    pub open spec fn string_order_contract() -> bool {
        vstd::laws_cmp::obeys_cmp::<String>()
    }
    // ENV-STRING-KEY: standard String hashing/equality obey vstd's key model.
    // This library contract is explicit: vstd does not discharge it for String.
    pub open spec fn string_hash_contract() -> bool {
        vstd::std_specs::hash::obeys_key_model::<String>()
    }
    // ENV-STRING-KEY: expose the content identity required by vstd's opaque
    // String key model. This is conditional, not a straw theorem about allocators.
    pub open spec fn string_content_contract() -> bool {
        forall|left: String, right: String| left@ == right@ ==> left == right
    }
    // ENV-GC-IO / ENV-SYNC: successful Unlink means current absence (including
    // trustworthy NotFound), then successful SyncParent persists that absence.
    // Failure may have partial effects; the production syscall adapter is not
    // proved to implement this relation. No other client recreates this pack.
    pub open spec fn gc_io_response(s: GcState, success: bool, after_present: bool,
        after_durable: bool, n: GcState) -> bool {
        if success {
            if s.controller.completed == 0 {
                !n.present && n.durable_present == s.durable_present
            } else {
                n.present == s.present && n.durable_present == s.present
            }
        } else {
            n.present == after_present && n.durable_present == after_durable
        }
    }
    pub open spec fn gc_crash_response(s: GcState, restored: bool, n: GcState) -> bool {
        n.durable_present == s.durable_present
            && (restored == s.present || restored == s.durable_present) && n.present == restored
    }
    // ENV-STABILIZE: no crashes or failed modeled I/O after some finite index.
    // No bound on when that index occurs, nor on elapsed wall-clock time.
    pub open spec fn gc_stable_from(events: spec_fn(nat) -> GcEvent, at: nat) -> bool {
        forall|i: nat| #![trigger events(i)] i >= at ==> !matches!(events(i), GcEvent::Crash { .. })
            && (matches!(events(i), GcEvent::Io { .. }) ==> events(i)->success)
    }
    pub open spec fn gc_eventually_stable(events: spec_fn(nat) -> GcEvent) -> bool {
        exists|at: nat| gc_stable_from(events, at)
    }
    // ENV-FAIR / ENV-GC-RETRY: each continuously enabled I/O or new attempt
    // eventually executes. Fair retry is an external caller obligation.
    pub open spec fn gc_fair_at(trace: spec_fn(nat) -> GcState, events: spec_fn(nat) -> GcEvent, start: nat) -> bool {
            ((forall|i: nat| #![trigger trace(i)] i >= start ==> io_enabled(trace(i))) ==>
                exists|i: nat| #![trigger events(i)] i >= start && matches!(events(i), GcEvent::Io { .. }))
            && ((forall|i: nat| #![trigger trace(i)] i >= start ==> retry_enabled(trace(i))) ==>
                exists|i: nat| #![trigger events(i)] i >= start && matches!(events(i), GcEvent::Retry))
    }
    pub open spec fn gc_weak_fair(trace: spec_fn(nat) -> GcState, events: spec_fn(nat) -> GcEvent) -> bool {
        forall|start: nat| gc_fair_at(trace, events, start)
    }
    // ENV-VERUS-PROGRESS: modeled stage effects eventually stop failing/crashing.
    // Valid inputs, eventual lock/I/O completion and faithful effect reporting
    // are adapter obligations; this is not a theorem about a filesystem alone.
    pub open spec fn control_stable_from(events: spec_fn(nat) -> RetryEvent, at: nat) -> bool {
        forall|i: nat| #![trigger events(i)] i >= at ==> !matches!(events(i), RetryEvent::Crash)
            && (matches!(events(i), RetryEvent::Io { .. }) ==> events(i)->success)
    }
    pub open spec fn control_eventually_stable(events: spec_fn(nat) -> RetryEvent) -> bool {
        exists|at: nat| control_stable_from(events, at)
    }
    pub open spec fn control_fair_at(trace: spec_fn(nat) -> ProgressView, events: spec_fn(nat) -> RetryEvent,
        limit: nat, start: nat) -> bool {
        ((forall|i: nat| #![trigger trace(i)] i >= start ==> control_io_enabled(trace(i), limit)) ==>
            exists|i: nat| #![trigger events(i)] i >= start && matches!(events(i), RetryEvent::Io { .. }))
        && ((forall|i: nat| #![trigger trace(i)] i >= start ==> control_retry_enabled(trace(i))) ==>
            exists|i: nat| #![trigger events(i)] i >= start && matches!(events(i), RetryEvent::Retry))
    }
    pub open spec fn control_weak_fair(trace: spec_fn(nat) -> ProgressView, events: spec_fn(nat) -> RetryEvent, limit: nat) -> bool {
        forall|start: nat| control_fair_at(trace, events, limit, start)
    }
    // ENV-VERUS-CATALOG: real operations are assumed to produce only these
    // serialized durable model steps. The Rust I/O adapter has no such proof.
    pub open spec fn serialized_catalog_history(trace: spec_fn(nat) -> CatalogView) -> bool {
        forall|i: nat| #![trigger trace(i)] catalog_step(trace(i), trace(i + 1))
    }
    // ENV-CROSS-COMPOSITION: immutable payloads start durable; crashes may
    // restore unsynced deletions, but may not erase a still-present durable name.
    pub open spec fn composition_restored_names(current: Set<nat>, durable: Set<nat>, restored: Set<nat>) -> bool {
        current.subset_of(restored) && restored.subset_of(durable)
    }
    pub open spec fn composition_synced_names(durable: Set<nat>, pack: nat) -> Set<nat> {
        durable.remove(pack)
    }

}
