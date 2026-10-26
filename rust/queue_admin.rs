//! Ordered submission and scheduling-counter effects in actual queue replay.
#[cfg(straw_verus)]
use crate::queue_task;
use crate::{queue_reader, queue_receipt, queue_state, queue_transition};
use serde_json::Value;
use serde_json::value::Index;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error {
    Task(queue_state::Error),
    Transition(queue_transition::Error),
    Receipt(queue_receipt::Error),
    Reader(queue_reader::Error),
    Array(&'static str),
    State,
    Order,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (order as int + count as int <= usize::MAX),
))]
pub fn has_capacity(order: usize, count: usize) -> bool {
    order.checked_add(count).is_some()
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn submit_one(before: Value, after: Value, order: usize, next: usize, spec: Value, result: Result<(), Error>) -> bool {
        if order == usize::MAX { result == Err(Error::Order) && after == before && next == order }
        else { match result {
            Ok(()) => next as int == order as int + 1 && queue_state::submission_result(before, after, spec, order as u64, Ok(())),
            Err(Error::Task(error)) => next == order && queue_state::submission_result(before, after, spec, order as u64, Err(error)),
            _ => false,
        } }
    }
    pub open spec fn trace(before: Value, after: Value, order: usize, next: usize, specs: Seq<Value>, states: Seq<Value>) -> bool {
        order as int + specs.len() == next as int && states.len() == specs.len() + 1
            && states[0] == before && states.last() == after
            && forall|i: int| 0 <= i < specs.len() ==> submit_one(states[i], states[i + 1],
                (order as int + i) as usize, (order as int + i + 1) as usize, specs[i], Ok(()))
    }
    pub closed spec fn submitted(before: Value, after: Value, order: usize, next: usize, specs: Seq<Value>) -> bool {
        exists|states: Seq<Value>| trace(before, after, order, next, specs, states)
    }
    pub proof fn submitted_empty(state: Value, order: usize)
        ensures submitted(state, state, order, order, Seq::empty()),
    {
        reveal(submitted);
        assert(trace(state, state, order, order, Seq::empty(), seq![state]));
    }
    pub proof fn submitted_push(before: Value, middle: Value, after: Value, order: usize, current: usize, next: usize, specs: Seq<Value>, spec: Value)
        requires submitted(before, middle, order, current, specs), submit_one(middle, after, current, next, spec, Ok(())),
        ensures submitted(before, after, order, next, specs.push(spec)),
    {
        reveal(submitted);
        let states = choose|states: Seq<Value>| trace(before, middle, order, current, specs, states);
        assert forall|i: int| 0 <= i < specs.push(spec).len() implies submit_one(states.push(after)[i], states.push(after)[i + 1],
            (order as int + i) as usize, (order as int + i + 1) as usize, specs.push(spec)[i], Ok(())) by {
            if i < specs.len() { assert(submit_one(states[i], states[i + 1], (order as int + i) as usize, (order as int + i + 1) as usize, specs[i], Ok(()))); }
            else { assert(states[i] == middle); }
        }
        assert(trace(before, after, order, next, specs.push(spec), states.push(after)));
    }
    pub open spec fn batch_result(before: Value, after: Value, order: usize, next: usize, specs: Seq<Value>, result: Result<(), Error>) -> bool {
        match result {
            Ok(()) => submitted(before, after, order, next, specs),
            Err(error) => exists|count: nat| count < specs.len()
                && submitted(before, after, order, next, specs.take(count as int))
                && submit_one(after, after, next, next, specs[count as int], Err(error)),
        }
    }
    pub proof fn submission_trace(before: Value, after: Value, order: usize, next: usize, specs: Seq<Value>) -> (states: Seq<Value>)
        requires submitted(before, after, order, next, specs),
        ensures trace(before, after, order, next, specs, states),
    {
        reveal(submitted);
        choose|states: Seq<Value>| trace(before, after, order, next, specs, states)
    }
    pub proof fn capacity_rules_out_order_error(before: Value, after: Value, order: usize, next: usize, specs: Seq<Value>, result: Result<(), Error>)
        requires batch_result(before, after, order, next, specs, result), order as int + specs.len() <= usize::MAX,
        ensures result != Err(Error::Order),
    {
        if result == Err(Error::Order) {
            let count = choose|count: nat| count < specs.len()
                && submitted(before, after, order, next, specs.take(count as int))
                && submit_one(after, after, next, next, specs[count as int], Err(Error::Order));
            let states = submission_trace(before, after, order, next, specs.take(count as int));
            assert(next == usize::MAX);
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures submit_one(*old(state), *final(state), *old(order), *final(order), *spec, result),
))]
pub fn submit(state: &mut Value, order: &mut usize, spec: &Value) -> Result<(), Error> {
    let Some(next) = order.checked_add(1) else {
        return Err(Error::Order);
    };
    if let Err(error) = queue_state::submit(state, spec, *order as u64) {
        return Err(Error::Task(error));
    }
    *order = next;
    Ok(())
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures batch_result(*old(state), *final(state), *old(order), *final(order), specs@, result),
))]
// The explicit error branch carries the erased completed-prefix witness.
#[allow(clippy::question_mark)]
pub fn submit_many(state: &mut Value, order: &mut usize, specs: &[Value]) -> Result<(), Error> {
    let mut index = 0usize;
    #[cfg(straw_verus)]
    proof! { assert(specs@.take(0) =~= Seq::<Value>::empty()); submitted_empty(*state, *order); }
    #[cfg_attr(straw_verus, verus_spec(
        invariant index <= specs.len(), submitted(*old(state), *state, *old(order), *order, specs@.take(index as int)),
            crate::environment::string_order_contract(),
        decreases specs.len() - index,
    ))]
    while index < specs.len() {
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost((*state, *order)));
        if let Err(error) = submit(state, order, &specs[index]) {
            #[cfg(straw_verus)]
            proof! { assert(exists|count: nat| count == index && count < specs.len()
            && submitted(*old(state), *state, *old(order), *order, specs@.take(count as int))
            && submit_one(*state, *state, *order, *order, specs@[count as int], Err(error))); }
            return Err(error);
        }
        #[cfg(straw_verus)]
        proof! {
            submitted_push(*old(state), before@.0, *state, *old(order), before@.1, *order, specs@.take(index as int), specs@[index as int]);
            assert(specs@.take(index as int + 1) == specs@.take(index as int).push(specs@[index as int]));
        }
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! { assert(specs@.take(index as int) == specs@); }
    Ok(())
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn progress_result<'a>(before: Value, after: Value, order: usize, next: usize, event: Value, yielding: bool, result: Result<&'a String, Error>) -> bool {
        if yielding && order == usize::MAX { result == Err(Error::Order) && after == before && next == order }
        else { match result {
            Ok(id) => queue_state::field_id(event, "task_id"@) == Some(*id)
                && queue_transition::progress_result(before, after, event, yielding, order as u64, Ok(()))
                && next as int == order as int + if yielding { 1int } else { 0int },
            Err(Error::Transition(error)) => next == order
                && queue_transition::progress_result(before, after, event, yielding, order as u64, Err(error)),
            _ => false,
        } }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures progress_result(*old(state), *final(state), *old(order), *final(order), *event, yielding, result),
))]
pub fn progress<'a>(
    state: &mut Value,
    order: &mut usize,
    event: &'a Value,
    yielding: bool,
) -> Result<&'a String, Error> {
    let next = if yielding {
        match order.checked_add(1) {
            Some(next) => next,
            None => return Err(Error::Order),
        }
    } else {
        *order
    };
    match queue_transition::progress(state, event, yielding, *order as u64) {
        Ok(id) => {
            *order = next;
            Ok(id)
        }
        Err(error) => Err(Error::Transition(error)),
    }
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn array_field(value: Value, name: Seq<char>) -> Option<Seq<Value>> {
        match crate::request::field_or_null(value, name) { Value::Array(items) => Some(items@), _ => None }
    }
    pub open spec fn producers(state: Value) -> Value { crate::request::field_or_null(state, "producers"@) }
    pub open spec fn producer_result(before: Value, after: Value, event: Value, result: Result<(), Error>) -> bool {
        if crate::json_specs::field(before, "producers"@).is_none() { result == Err(Error::State) && after == before }
        else { match queue_receipt::producer(event) {
            None => result.is_ok() && after == before,
            Some(id) => match result {
                Err(Error::Receipt(queue_receipt::Error::State)) => !(producers(before) is Object) && after == before,
                Ok(()) => producers(before) is Object && producers(after) is Object
                    && queue_task::unchanged(before, after, set!["producers"@])
                    && crate::json_update::fields(producers(after)).contains_key(id)
                    && crate::json_update::fields(producers(after)) == crate::json_update::fields(producers(before)).insert(id, crate::json_update::fields(producers(after))[id])
                    && crate::queue_metadata::same_value(crate::json_update::fields(producers(after))[id], crate::request::field_or_null(event, "producer_state"@)),
                _ => false,
            }
        } }
    }
    pub closed spec fn submission_tail(before: Value, middle: Value, after: Value, order: usize, next: usize, event: Value, result: Result<(), Error>) -> bool {
        array_field(event, "tasks"@).is_some()
            && batch_result(before, middle, order, next, array_field(event, "tasks"@)->Some_0, Ok(()))
            && producer_result(middle, after, event, result)
    }
    pub open spec fn submission_result(before: Value, after: Value, order: usize, next: usize, event: Value, result: Result<(), Error>) -> bool {
        match array_field(event, "tasks"@) {
            None => result == Err(Error::Array("tasks")) && after == before && next == order,
            Some(specs) => (exists|middle: Value| submission_tail(before, middle, after, order, next, event, result))
                || result.is_err() && batch_result(before, after, order, next, specs, result),
        }
    }
    pub open spec fn epoch_result(before: Value, after: Value, epoch: String, next: String, event: Value, result: Result<(), Error>) -> bool {
        match queue_state::field_id(event, "epoch"@) {
            None => result == Err(Error::Task(queue_state::Error::Identity("epoch"))) && next == epoch && after == before,
            Some(id) => next == id && match array_field(event, "revoked"@) {
                None => result == Err(Error::Array("revoked")) && after == before,
                Some(ids) => match result {
                    Ok(()) => queue_state::many_result(before, after, ids, queue_state::Change::Retry, Ok(())),
                    Err(Error::Task(error)) => queue_state::many_result(before, after, ids, queue_state::Change::Retry, Err(error)),
                    _ => false,
                }
            }
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures match result {
        Ok(items) => array_field(*event, name@) == Some(items@),
        Err(error) => array_field(*event, name@).is_none() && error == Error::Array(name),
    },
))]
pub(crate) fn array<'a>(event: &'a Value, name: &'static str) -> Result<&'a [Value], Error> {
    match name.index_into(event) {
        Some(Value::Array(items)) => Ok(items),
        _ => Err(Error::Array(name)),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures producer_result(*old(state), *final(state), *event, result),
))]
fn continue_producer(state: &mut Value, event: &Value) -> Result<(), Error> {
    let Some(producers) = "producers".index_into_mut(state) else {
        return Err(Error::State);
    };
    match queue_receipt::continue_producer(producers, event) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Receipt(error)),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures submission_result(*old(state), *final(state), *old(order), *final(order), *event, result),
))]
pub fn submitted_event(state: &mut Value, order: &mut usize, event: &Value) -> Result<(), Error> {
    let specs = array(event, "tasks")?;
    submit_many(state, order, specs)?;
    #[cfg(straw_verus)]
    let middle = verus_exec_expr!(Ghost(*state));
    let result = continue_producer(state, event);
    #[cfg(straw_verus)]
    proof! {
        reveal(submission_tail);
        assert(submission_tail(*old(state), middle@, *state, *old(order), *order, *event, result));
    }
    result
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures epoch_result(*old(state), *final(state), *old(epoch), *final(epoch), *event, result),
))]
pub fn apply_epoch(state: &mut Value, epoch: &mut String, event: &Value) -> Result<(), Error> {
    let id = match queue_state::identity(event, "epoch") {
        Ok(id) => id,
        Err(error) => return Err(Error::Task(error)),
    };
    *epoch = id.clone();
    let revoked = array(event, "revoked")?;
    match queue_state::edit_many(state, revoked, queue_state::Change::Retry) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Task(error)),
    }
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn requests(state: Value) -> Value { crate::request::field_or_null(state, "requests"@) }
    pub open spec fn remember_result(before: Value, after: Value, event: Value, result: Result<(), Error>) -> bool {
        match (requests(before), requests(after), result) {
            (Value::Object(old_table), Value::Object(new_table), Ok(())) => queue_task::unchanged(before, after, set!["requests"@])
                && crate::request::ledger(new_table) == crate::request::step(crate::request::ledger(old_table), event),
            (_, _, Err(Error::State)) => !(requests(before) is Object) && after == before,
            _ => false,
        }
    }
    pub open spec fn done_result(before: Value, after: Value, event: Value, result: Result<(), Error>) -> bool {
        match array_field(event, "leases"@) {
            None => result == Err(Error::Array("leases")) && after == before,
            Some(leases) => if crate::json_specs::field(before, "readers"@).is_none() { result == Err(Error::State) && after == before }
                else { queue_task::unchanged(before, after, set!["readers"@]) && match result {
                    Ok(()) => queue_reader::done_result(queue_transition::readers(before), queue_transition::readers(after), leases, Ok(())),
                    Err(Error::Reader(error)) => queue_reader::done_result(queue_transition::readers(before), queue_transition::readers(after), leases, Err(error)),
                    _ => false,
                } },
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures remember_result(*old(state), *final(state), *event, result),
))]
pub fn remember(state: &mut Value, event: &Value) -> Result<(), Error> {
    let Some(Value::Object(requests)) = "requests".index_into_mut(state) else {
        return Err(Error::State);
    };
    crate::request::remember(requests, event);
    Ok(())
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures done_result(*old(state), *final(state), *event, result),
))]
pub fn reader_done(state: &mut Value, event: &Value) -> Result<(), Error> {
    let leases = array(event, "leases")?;
    let Some(readers) = "readers".index_into_mut(state) else {
        return Err(Error::State);
    };
    match queue_reader::done(readers, leases) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Reader(error)),
    }
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn retired_result(before: Value, after: Value, event: Value, result: Result<(), Error>) -> bool {
        let readers = queue_transition::readers(before);
        match result {
            Ok(()) => queue_task::unchanged(before, after, set!["readers"@])
                && queue_reader::retired(readers, queue_transition::readers(after), crate::request::field_or_null(event, "worker_id"@)),
            Err(Error::State) => crate::json_specs::field(before, "readers"@).is_none() && after == before,
            Err(Error::Reader(queue_reader::Error::State)) => crate::json_specs::field(before, "readers"@).is_some()
                && !(readers is Object) && after == before,
            _ => false,
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures retired_result(*old(state), *final(state), *event, result),
))]
pub fn reader_retired(state: &mut Value, event: &Value) -> Result<(), Error> {
    let worker = "worker_id".index_into(event).unwrap_or(&Value::Null);
    let Some(readers) = "readers".index_into_mut(state) else {
        return Err(Error::State);
    };
    match queue_reader::retire(readers, worker) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Reader(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state() -> Value {
        json!({"tasks":{},"producers":{"peer":"old"},"readers":{"attempt":{"roots":["input"]},"peer":{"roots":["other"]}},"requests":{},"checkpoints":{"keep":true}})
    }

    #[test]
    fn submission_prefix_keeps_orders_and_defers_producer_until_the_batch_succeeds() {
        let mut value = state();
        let mut order = 7;
        let event = json!({"tasks":[{"task_id":"a","metadata":1},{"task_id":"a","metadata":2},{"task_id":false},{"task_id":"later"}],
            "producer_id":"producer","producer_state":{"position":4}});
        assert!(matches!(
            submitted_event(&mut value, &mut order, &event),
            Err(Error::Task(queue_state::Error::Identity("task_id")))
        ));
        assert_eq!(order, 9);
        assert_eq!(value["tasks"].as_object().unwrap().len(), 1);
        assert_eq!(value["tasks"]["a"]["spec"]["metadata"], 2);
        assert_eq!(value["tasks"]["a"]["pending_order"], 8);
        assert_eq!(value["producers"], json!({"peer":"old"}));
        assert_eq!(value["readers"], state()["readers"]);
        let event = json!({"tasks":[{"task_id":"last"}],"producer_id":"producer","producer_state":{"position":5}});
        submitted_event(&mut value, &mut order, &event).unwrap();
        assert_eq!(order, 10);
        assert_eq!(value["producers"]["producer"], event["producer_state"]);
        value["producers"] = Value::Bool(false);
        assert!(matches!(
            submitted_event(&mut value, &mut order, &event),
            Err(Error::Receipt(queue_receipt::Error::State))
        ));
        assert_eq!(order, 11);
        assert_eq!(value["tasks"]["last"]["pending_order"], 10);
    }

    #[test]
    fn exhausted_order_rejects_before_mutation_and_preserves_the_batch_prefix() {
        assert!(has_capacity(usize::MAX, 0));
        assert!(!has_capacity(usize::MAX, 1));
        assert!(has_capacity(usize::MAX - 1, 1));
        let mut value = state();
        let mut order = usize::MAX - 1;
        assert!(matches!(
            submit_many(
                &mut value,
                &mut order,
                &[json!({"task_id":"last"}), json!({"task_id":"overflow"})]
            ),
            Err(Error::Order)
        ));
        assert_eq!(order, usize::MAX);
        assert_eq!(value["tasks"].as_object().unwrap().len(), 1);
        assert_eq!(
            value["tasks"]["last"]["pending_order"],
            json!(usize::MAX - 1)
        );
        let before = value.clone();
        assert!(matches!(
            progress(&mut value, &mut order, &json!({"task_id":"last"}), true),
            Err(Error::Order)
        ));
        assert_eq!(value, before);
        progress(
            &mut value,
            &mut order,
            &json!({"task_id":"last", "input_ref":null}),
            false,
        )
        .unwrap();
        assert_eq!(order, usize::MAX);
        submit_many(&mut value, &mut order, &[]).unwrap();
    }

    #[test]
    fn epoch_and_reader_done_errors_expose_the_exact_applied_prefix() {
        let mut value = state();
        let mut order = 0;
        submit(
            &mut value,
            &mut order,
            &json!({"task_id":"a", "max_attempts":4}),
        )
        .unwrap();
        let mut epoch = "old".to_owned();
        assert!(matches!(
            apply_epoch(
                &mut value,
                &mut epoch,
                &json!({"epoch":"new","revoked":["a","a",false]})
            ),
            Err(Error::Task(queue_state::Error::Identity("task ID")))
        ));
        assert_eq!(epoch, "new");
        assert_eq!(value["tasks"]["a"]["failures"], 2);
        assert_eq!(value["readers"], state()["readers"]);
        let before = value.clone();
        assert!(matches!(
            apply_epoch(&mut value, &mut epoch, &json!({"epoch":"latest"})),
            Err(Error::Array("revoked"))
        ));
        assert_eq!(epoch, "latest");
        assert_eq!(value, before);
        assert!(matches!(
            reader_done(
                &mut value,
                &json!({"leases":[{"attempt_id":"attempt"},{"attempt_id":false}]})
            ),
            Err(Error::Reader(queue_reader::Error::Identity))
        ));
        assert_eq!(value["readers"], json!({"peer":{"roots":["other"]}}));
        assert_eq!(value["checkpoints"], before["checkpoints"]);
    }

    #[test]
    fn request_receipt_binding_preserves_business_fields_and_rejects_invalid_tables() {
        let event =
            json!({"request_key":"submit/request","request_digest":"digest","response":["a"]});
        let mut value = state();
        let mut expected = value.clone();
        expected["requests"]["submit/request"] = json!(["digest", ["a"]]);
        remember(&mut value, &event).unwrap();
        assert_eq!(value, expected);
        value["requests"] = Value::Bool(false);
        let before = value.clone();
        assert!(matches!(remember(&mut value, &event), Err(Error::State)));
        assert_eq!(value, before);
    }
}
