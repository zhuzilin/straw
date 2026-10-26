//! Business-event dispatch shared by native queue recovery and formal replay.
use crate::{
    queue_admin as admin, queue_metadata as metadata, queue_state, queue_transition as transition,
};
use serde_json::Value;
use serde_json::value::Index;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error {
    Admin(admin::Error),
    Transition(transition::Error),
    Metadata(metadata::Error),
    Unknown,
}

/// Native cache maintenance is separate from the business-state relation.
/// The callback has no access to the exclusively borrowed queue state.
#[cfg_attr(straw_verus, verus_verify)]
pub trait Deadlines {
    #[allow(clippy::ptr_arg)]
    fn remove(&mut self, id: &String);
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn item_result(before: Value, after: Value, item: Value, event: Value, leased: bool, result: Result<(), Error>) -> bool {
        if leased { match result {
            Ok(()) => transition::leased_result(before, after, item, Ok(())),
            Err(Error::Transition(error)) => transition::leased_result(before, after, item, Err(error)),
            _ => false,
        } } else { match queue_state::value_id(item) {
            None => result == Err(Error::Admin(admin::Error::Task(queue_state::Error::Identity("task ID")))) && after == before,
            Some(id) => match result {
                Ok(()) => transition::finish_result(before, after, id, queue_state::Change::Failed(&event),
                    crate::queue_event::text(event, "type"@) == Some("Failed"@), Ok(())),
                Err(Error::Transition(error)) => transition::finish_result(before, after, id, queue_state::Change::Failed(&event),
                    crate::queue_event::text(event, "type"@) == Some("Failed"@), Err(error)),
                _ => false,
            }
        } }
    }
    pub open spec fn trace(before: Value, after: Value, items: Seq<Value>, event: Value, leased: bool, states: Seq<Value>) -> bool {
        states.len() == items.len() + 1 && states[0] == before && states.last() == after
            && forall|i: int| 0 <= i < items.len() ==> item_result(states[i], states[i + 1], items[i], event, leased, Ok(()))
    }
    pub closed spec fn applied(before: Value, after: Value, items: Seq<Value>, event: Value, leased: bool) -> bool {
        exists|states: Seq<Value>| trace(before, after, items, event, leased, states)
    }
    pub proof fn empty(state: Value, event: Value, leased: bool)
        ensures applied(state, state, Seq::empty(), event, leased),
    {
        reveal(applied);
        assert(trace(state, state, Seq::empty(), event, leased, seq![state]));
    }
    pub proof fn push(before: Value, middle: Value, after: Value, items: Seq<Value>, item: Value, event: Value, leased: bool)
        requires applied(before, middle, items, event, leased), item_result(middle, after, item, event, leased, Ok(())),
        ensures applied(before, after, items.push(item), event, leased),
    {
        reveal(applied);
        let states = choose|states: Seq<Value>| trace(before, middle, items, event, leased, states);
        assert forall|i: int| 0 <= i < items.push(item).len() implies item_result(states.push(after)[i], states.push(after)[i + 1],
            items.push(item)[i], event, leased, Ok(())) by {
            if i < items.len() { assert(item_result(states[i], states[i + 1], items[i], event, leased, Ok(()))); }
            else { assert(states[i] == middle); }
        }
        assert(trace(before, after, items.push(item), event, leased, states.push(after)));
    }
    pub open spec fn batch_result(before: Value, after: Value, items: Seq<Value>, event: Value, leased: bool, result: Result<(), Error>) -> bool {
        match result {
            Ok(()) => applied(before, after, items, event, leased),
            Err(error) => exists|count: nat, middle: Value| count < items.len()
                && applied(before, middle, items.take(count as int), event, leased)
                && item_result(middle, after, items[count as int], event, leased, Err(error)),
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures item_result(*old(state), *final(state), *item, *event, leased, result),
))]
fn apply_item<D: Deadlines>(
    state: &mut Value,
    item: &Value,
    event: &Value,
    leased: bool,
    deadlines: &mut D,
) -> Result<(), Error> {
    if leased {
        let outcome = transition::leased(state, item);
        #[cfg(straw_verus)]
        proof! { assert(outcome->Ok_0 == ()); }
        match outcome {
            Ok(()) => Ok(()),
            Err(error) => Err(Error::Transition(error)),
        }
    } else {
        let id = match queue_state::task_id(item) {
            Ok(id) => id,
            Err(error) => return Err(Error::Admin(admin::Error::Task(error))),
        };
        let outcome = transition::failed(state, id, event);
        #[cfg(straw_verus)]
        proof! { assert(outcome->Ok_0 == ()); }
        match outcome {
            Ok(()) => {
                deadlines.remove(id);
                Ok(())
            }
            Err(error) => Err(Error::Transition(error)),
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures batch_result(*old(state), *final(state), items@, *event, leased, result),
))]
#[allow(clippy::question_mark)]
fn many<D: Deadlines>(
    state: &mut Value,
    items: &[Value],
    event: &Value,
    leased: bool,
    deadlines: &mut D,
) -> Result<(), Error> {
    let mut index = 0usize;
    #[cfg(straw_verus)]
    proof! { assert(items@.take(0) =~= Seq::<Value>::empty()); empty(*state, *event, leased); }
    #[cfg_attr(straw_verus, verus_spec(
        invariant index <= items.len(), applied(*old(state), *state, items@.take(index as int), *event, leased),
            crate::environment::string_order_contract(),
        decreases items.len() - index,
    ))]
    while index < items.len() {
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(*state));
        if let Err(error) = apply_item(state, &items[index], event, leased, deadlines) {
            #[cfg(straw_verus)]
            proof! { assert(exists|count: nat, middle: Value| count == index && middle == before@ && count < items.len()
            && applied(*old(state), middle, items@.take(count as int), *event, leased)
            && item_result(middle, *state, items@[count as int], *event, leased, Err(error))); }
            return Err(error);
        }
        #[cfg(straw_verus)]
        proof! {
            push(*old(state), before@, *state, items@.take(index as int), items@[index as int], *event, leased);
            assert(items@.take(index as int + 1) == items@.take(index as int).push(items@[index as int]));
        }
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! { assert(items@.take(index as int) == items@); }
    Ok(())
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn batch_event_result(before: Value, after: Value, event: Value, leased: bool, result: Result<(), Error>) -> bool {
        match admin::array_field(event, if leased { "leases"@ } else { "task_ids"@ }) {
            None => result == Err(Error::Admin(admin::Error::Array(if leased { "leases" } else { "task_ids" }))) && after == before,
            Some(items) => batch_result(before, after, items, event, leased, result),
        }
    }
    pub open spec fn admin_unit(result: Result<bool, Error>) -> Option<Result<(), admin::Error>> {
        match result { Ok(false) => Some(Ok(())), Err(Error::Admin(error)) => Some(Err(error)), _ => None }
    }
    pub open spec fn transition_unit(result: Result<bool, Error>) -> Option<Result<(), transition::Error>> {
        match result { Ok(false) => Some(Ok(())), Err(Error::Transition(error)) => Some(Err(error)), _ => None }
    }
    pub open spec fn unit(result: Result<bool, Error>) -> Result<(), Error> {
        match result { Ok(_) => Ok(()), Err(error) => Err(error) }
    }
    pub open spec fn body_result(before: Value, after: Value, epoch: String, new_epoch: String, order: usize, next: usize,
        sealed: bool, new_sealed: bool, event: Value, kind: Seq<char>, result: Result<bool, Error>) -> bool {
        (kind != "Epoch"@ ==> new_epoch == epoch)
        && (kind != "Submitted"@ && kind != "Yielded"@ && kind != "TaskProgress"@ ==> next == order)
        && (kind != "Sealed"@ ==> new_sealed == sealed)
        && if kind == "StagingHandoff"@ { after == before && result == Ok(false) }
        else if kind == "Epoch"@ { admin_unit(result).is_some() && admin::epoch_result(before, after, epoch, new_epoch, event, admin_unit(result)->Some_0) }
        else if kind == "Submitted"@ { admin_unit(result).is_some() && admin::submission_result(before, after, order, next, event, admin_unit(result)->Some_0) }
        else if kind == "Leased"@ || kind == "Expired"@ || kind == "Failed"@ {
            result != Ok(true) && batch_event_result(before, after, event, kind == "Leased"@, unit(result))
        }
        else if kind == "Cancelled"@ { transition_unit(result).is_some() && transition::cancelled_result(before, after, event, transition_unit(result)->Some_0) }
        else if kind == "Completed"@ { transition_unit(result).is_some() && transition::complete_result(before, after,
            crate::request::field_or_null(event, "receipt"@), crate::request::field_or_null(event, "lease"@), transition_unit(result)->Some_0) }
        else if kind == "Yielded"@ || kind == "TaskProgress"@ { match result {
            Ok(false) => exists|id: String| admin::progress_result(before, after, order, next, event, kind == "Yielded"@, Ok(&id)),
            Err(Error::Admin(error)) => admin::progress_result(before, after, order, next, event, kind == "Yielded"@, Err(error)),
            _ => false,
        } }
        else if kind == "Sealed"@ { new_sealed && after == before && result == Ok(false) }
        else if kind == "ConsumerState"@ || kind == "BatchPlanned"@ || kind == "BatchReady"@ || kind == "Checkpoint"@
            || kind == "CheckpointReleased"@ || kind == "RetentionAdvanced"@ { match result {
                Ok(true) => metadata::outcome(before, after, event, Ok(())),
                Err(Error::Metadata(error)) => metadata::outcome(before, after, event, Err(error)),
                _ => false,
            } }
        else if kind == "ReaderDone"@ { admin_unit(result).is_some() && admin::done_result(before, after, event, admin_unit(result)->Some_0) }
        else if kind == "ReaderRetired"@ { admin_unit(result).is_some() && admin::retired_result(before, after, event, admin_unit(result)->Some_0) }
        else { result == Err(Error::Unknown) && after == before }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures batch_event_result(*old(state), *final(state), *event, leased, result),
))]
fn batch<D: Deadlines>(
    state: &mut Value,
    event: &Value,
    leased: bool,
    deadlines: &mut D,
) -> Result<(), Error> {
    let name = if leased { "leases" } else { "task_ids" };
    let items = match admin::array(event, name) {
        Ok(items) => items,
        Err(error) => return Err(Error::Admin(error)),
    };
    many(state, items, event, leased, deadlines)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(), crate::queue_event::text(*event, "type"@) == Some(kind@),
    ensures body_result(*old(state), *final(state), *old(epoch), *final(epoch), *old(order), *final(order),
        *old(sealed), *final(sealed), *event, kind@, result),
))]
#[allow(clippy::too_many_arguments)]
pub fn body<D: Deadlines>(
    state: &mut Value,
    epoch: &mut String,
    order: &mut usize,
    sealed: &mut bool,
    event: &Value,
    kind: &str,
    deadlines: &mut D,
) -> Result<bool, Error> {
    if kind == "StagingHandoff" {
    } else if kind == "Epoch" {
        match admin::apply_epoch(state, epoch, event) {
            Ok(()) => {}
            Err(e) => return Err(Error::Admin(e)),
        }
    } else if kind == "Submitted" {
        let outcome = admin::submitted_event(state, order, event);
        #[cfg(straw_verus)]
        proof! { assert(outcome->Ok_0 == ()); }
        match outcome {
            Ok(()) => {}
            Err(e) => return Err(Error::Admin(e)),
        }
    } else if kind == "Leased" {
        batch(state, event, true, deadlines)?;
    } else if kind == "Expired" || kind == "Failed" {
        batch(state, event, false, deadlines)?;
    } else if kind == "Cancelled" {
        match transition::cancelled(state, event) {
            Ok(id) => deadlines.remove(id),
            Err(e) => return Err(Error::Transition(e)),
        }
    } else if kind == "Yielded" || kind == "TaskProgress" {
        let yielding = kind == "Yielded";
        match admin::progress(state, order, event, yielding) {
            Ok(id) => {
                if yielding {
                    deadlines.remove(id);
                }
                #[cfg(straw_verus)]
                proof! { assert(exists|name: String| name == *id && admin::progress_result(*old(state), *state, *old(order), *order, *event, yielding, Ok(&name))); }
            }
            Err(e) => return Err(Error::Admin(e)),
        }
    } else if kind == "Completed" {
        let receipt = "receipt".index_into(event).unwrap_or(&Value::Null);
        let lease = "lease".index_into(event).unwrap_or(&Value::Null);
        match transition::complete(state, receipt, lease) {
            Ok(id) => deadlines.remove(id),
            Err(e) => return Err(Error::Transition(e)),
        }
    } else if kind == "Sealed" {
        *sealed = true;
    } else if kind == "ConsumerState"
        || kind == "BatchPlanned"
        || kind == "BatchReady"
        || kind == "Checkpoint"
        || kind == "CheckpointReleased"
        || kind == "RetentionAdvanced"
    {
        return match metadata::apply(state, event) {
            Ok(()) => Ok(true),
            Err(e) => Err(Error::Metadata(e)),
        };
    } else if kind == "ReaderDone" {
        match admin::reader_done(state, event) {
            Ok(()) => {}
            Err(e) => return Err(Error::Admin(e)),
        }
    } else if kind == "ReaderRetired" {
        match admin::reader_retired(state, event) {
            Ok(()) => {}
            Err(e) => return Err(Error::Admin(e)),
        }
    } else {
        return Err(Error::Unknown);
    }
    Ok(false)
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn final_result(before: Value, after: Value, event: Value, recorded: bool, result: Result<(), admin::Error>) -> bool {
        if recorded { after == before && result.is_ok() }
        else { admin::remember_result(before, after, event, result) }
    }
    pub proof fn receipt_flag(before: Value, after: Value, epoch: String, new_epoch: String, order: usize, next: usize,
        sealed: bool, new_sealed: bool, event: Value, kind: Seq<char>, recorded: bool)
        requires body_result(before, after, epoch, new_epoch, order, next, sealed, new_sealed, event, kind, Ok(recorded)),
        ensures recorded == (kind == "ConsumerState"@ || kind == "BatchPlanned"@ || kind == "BatchReady"@ || kind == "Checkpoint"@
            || kind == "CheckpointReleased"@ || kind == "RetentionAdvanced"@),
    {}
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures final_result(*old(state), *final(state), *event, recorded, result),
))]
pub fn finish(state: &mut Value, event: &Value, recorded: bool) -> Result<(), admin::Error> {
    if recorded {
        Ok(())
    } else {
        admin::remember(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct Removed(Vec<String>);
    impl Deadlines for Removed {
        fn remove(&mut self, id: &String) {
            self.0.push(id.clone());
        }
    }
    fn state() -> Value {
        json!({"tasks":{},"readers":{},"requests":{},"producers":{},"submissions":{},"commits":[],
            "consumers":{},"batches":{},"checkpoints":{"peer":{"state_ref":"checkpoint-root"}}})
    }
    fn apply(value: &mut Value, event: &Value, removed: &mut Removed) -> Result<bool, Error> {
        body(
            value,
            &mut "epoch".to_owned(),
            &mut 7,
            &mut false,
            event,
            event["type"].as_str().unwrap(),
            removed,
        )
    }

    #[test]
    fn failed_batch_preserves_the_successful_prefix_and_the_failing_items_reader_effect() {
        let mut value = state();
        for id in ["a", "b", "peer"] {
            queue_state::submit(
                &mut value,
                &json!({"task_id":id,"max_attempts":5,"input_ref":id}),
                0,
            )
            .unwrap();
            transition::leased(
                &mut value,
                &json!({"task_id":id,"attempt_id":id,"worker_id":id}),
            )
            .unwrap();
        }
        value["tasks"]["b"]["failures"] = json!(u64::MAX);
        let before = value.clone();
        let mut removed = Removed::default();
        assert!(matches!(
            apply(
                &mut value,
                &json!({"type":"Failed","task_ids":["a","b","peer"],"retryable":true}),
                &mut removed
            ),
            Err(Error::Transition(transition::Error::Task(
                queue_state::Error::Task(crate::queue_task::Error::Counter)
            )))
        ));
        assert_eq!(removed.0, vec!["a"]);
        assert_eq!(value["tasks"]["a"]["failures"], 1);
        assert_eq!(value["tasks"]["b"], before["tasks"]["b"]);
        assert_eq!(value["tasks"]["peer"], before["tasks"]["peer"]);
        assert_eq!(value["readers"], json!({"peer":before["readers"]["peer"]}));
        assert_eq!(value["checkpoints"], before["checkpoints"]);
        assert_eq!(value["requests"], before["requests"]);
    }

    #[test]
    fn lease_batch_stops_after_the_partial_failing_item_and_expiry_keeps_readers() {
        let mut value = state();
        for id in ["a", "b", "later"] {
            queue_state::submit(
                &mut value,
                &json!({"task_id":id,"max_attempts":5,"input_ref":id}),
                0,
            )
            .unwrap();
        }
        let mut removed = Removed::default();
        assert!(matches!(
            apply(
                &mut value,
                &json!({"type":"Leased","leases":[
            {"task_id":"a","attempt_id":"first"},{"task_id":"b"},{"task_id":"later","attempt_id":"last"}]}),
                &mut removed
            ),
            Err(Error::Transition(transition::Error::Reader(
                crate::queue_reader::Error::Identity
            )))
        ));
        assert_eq!(value["tasks"]["a"]["state"], "leased");
        assert_eq!(value["tasks"]["b"]["state"], "leased");
        assert_eq!(value["tasks"]["later"]["state"], "pending");
        let roots = value["readers"].clone();
        apply(
            &mut value,
            &json!({"type":"Expired","task_ids":["a","a"]}),
            &mut removed,
        )
        .unwrap();
        assert_eq!(removed.0, vec!["a", "a"]);
        assert_eq!(value["tasks"]["a"]["failures"], 2);
        assert_eq!(value["readers"], roots);
    }

    #[test]
    fn worker_retirement_preserves_peer_records_and_other_queue_roots() {
        let mut value = state();
        value["readers"] = json!({
            "a":{"lease":{"worker_id":"retiring"},"roots":["old","middle","latest"]},
            "b":{"lease":{"worker_id":"retiring"},"roots":["shared"]},
            "peer":{"lease":{"worker_id":"peer"},"roots":["shared","peer-only"]},
            "absent-worker":{"roots":["unknown"]}
        });
        let mut expected = value.clone();
        expected["readers"].as_object_mut().unwrap().remove("a");
        expected["readers"].as_object_mut().unwrap().remove("b");
        let event = json!({"type":"ReaderRetired","worker_id":"retiring"});
        let mut removed = Removed::default();
        assert!(!apply(&mut value, &event, &mut removed).unwrap());
        assert_eq!(value, expected);
        assert!(removed.0.is_empty());
        apply(&mut value, &event, &mut removed).unwrap();
        assert_eq!(value, expected);
        // Retain native Value indexing/equality semantics for malformed history.
        apply(&mut value, &json!({"type":"ReaderRetired"}), &mut removed).unwrap();
        assert!(value["readers"]["absent-worker"].is_null());
        assert_eq!(value["readers"]["peer"], expected["readers"]["peer"]);
        value["readers"] = Value::Bool(false);
        let before = value.clone();
        assert!(matches!(
            apply(&mut value, &event, &mut removed),
            Err(Error::Admin(admin::Error::Reader(
                crate::queue_reader::Error::State
            )))
        ));
        assert_eq!(value, before);
    }

    #[test]
    fn sealed_and_metadata_dispatch_preserve_other_controls_and_record_receipts_once() {
        let mut value = state();
        let before = value.clone();
        let (mut epoch, mut order, mut sealed) = ("epoch".to_owned(), 11, false);
        let mut removed = Removed::default();
        for kind in ["StagingHandoff", "Sealed"] {
            assert!(
                !body(
                    &mut value,
                    &mut epoch,
                    &mut order,
                    &mut sealed,
                    &json!({"type":kind}),
                    kind,
                    &mut removed
                )
                .unwrap()
            );
            assert_eq!(value, before);
        }
        assert!(sealed);
        let event = json!({"type":"ConsumerState","consumer_id":"training","state":{"processed_cursor":4},
            "request_key":"state/request","request_digest":"digest","response":null});
        assert!(
            body(
                &mut value,
                &mut epoch,
                &mut order,
                &mut sealed,
                &event,
                "ConsumerState",
                &mut removed
            )
            .unwrap()
        );
        assert_eq!(value["consumers"]["training"], event["state"]);
        assert_eq!(value["requests"]["state/request"], json!(["digest", null]));
        assert_eq!((epoch.as_str(), order, sealed), ("epoch", 11, true));
        let before = value.clone();
        assert!(matches!(
            apply(&mut value, &json!({"type":"Unknown"}), &mut removed),
            Err(Error::Unknown)
        ));
        assert_eq!(value, before);
    }
}
