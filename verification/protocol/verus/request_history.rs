//! Arbitrary finite histories of the actual request-receipt transition.
//! Admission, WAL durability and complete coordinator business replay are
//! distinct obligations; this does not claim that native calls satisfy them.
use serde_json::Value;
use vstd::prelude::*;
use crate::request::{Cached, event_key, event_receipt, step};

verus! {
    pub closed spec fn run(before: Map<String, Cached>, events: Seq<Value>) -> Map<String, Cached>
        decreases events.len(),
    {
        if events.len() == 0 { before }
        else { step(run(before, events.drop_last()), events.last()) }
    }
    pub open spec fn admitted(before: Map<String, Cached>, event: Value) -> bool {
        match event_key(event) {
            None => true,
            Some(key) => !before.contains_key(key) || before[key] == Cached::Receipt(event_receipt(event)),
        }
    }
    pub closed spec fn history(before: Map<String, Cached>, events: Seq<Value>) -> bool
        decreases events.len(),
    {
        events.len() == 0 || (history(before, events.drop_last())
            && admitted(run(before, events.drop_last()), events.last()))
    }
    pub proof fn append(before: Map<String, Cached>, events: Seq<Value>, event: Value)
        ensures run(before, events.push(event)) == step(run(before, events), event),
    { reveal(run); assert(events.push(event).drop_last() == events); }

    pub proof fn duplicate_is_idempotent(before: Map<String, Cached>, event: Value)
        ensures step(step(before, event), event) == step(before, event),
    {}

    pub proof fn other_requests_unchanged(before: Map<String, Cached>, event: Value, key: String)
        requires event_key(event) != Some(key),
        ensures step(before, event).contains_key(key) == before.contains_key(key),
            before.contains_key(key) ==> step(before, event)[key] == before[key],
    {}

    pub proof fn receipt_survives_history(before: Map<String, Cached>, events: Seq<Value>, key: String)
        requires history(before, events), before.contains_key(key),
        ensures run(before, events).contains_key(key) && run(before, events)[key] == before[key],
        decreases events.len(),
    {
        reveal(history); reveal(run);
        if events.len() > 0 {
            receipt_survives_history(before, events.drop_last(), key);
            if event_key(events.last()) != Some(key) {
                other_requests_unchanged(run(before, events.drop_last()), events.last(), key);
            }
        }
    }

    pub proof fn replay_chunks(before: Map<String, Cached>, first: Seq<Value>, second: Seq<Value>)
        ensures run(run(before, first), second) == run(before, first + second),
        decreases second.len(),
    {
        reveal(run);
        if second.len() > 0 {
            replay_chunks(before, first, second.drop_last());
            assert((first + second).drop_last() == first + second.drop_last());
            assert((first + second).last() == second.last());
        } else { assert(first + second == first); }
    }
}
