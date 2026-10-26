//! Bind task transitions to their actual queue table and execute ordered replay.
use crate::queue_task::{self, Terminal};
use serde_json::Value;
use serde_json::value::Index;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy)]
pub enum Change<'a> {
    Retry,
    Lease(&'a Value),
    Failed(&'a Value),
    Terminal(Terminal),
    Progress {
        event: &'a Value,
        yielding: bool,
        order: u64,
    },
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error {
    Identity(&'static str),
    Target,
    Task(queue_task::Error),
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn tasks(state: Value) -> Value { crate::request::field_or_null(state, "tasks"@) }
    pub open spec fn table(state: Value) -> vstd::map::Map<String, Value> { crate::json_update::fields(tasks(state)) }
    pub open spec fn target(state: Value, id: String) -> bool { tasks(state) is Object && table(state).contains_key(id) }
    pub open spec fn field_id(value: Value, field: Seq<char>) -> Option<String> {
        match crate::json_specs::field(value, field) { Some(Value::String(id)) => Some(id), _ => None }
    }
    pub open spec fn fault(before: Value, change: Change) -> Option<queue_task::Error> {
        if !(before is Object) { Some(queue_task::Error::Task) }
        else { match change {
            Change::Retry => if queue_task::count(before, "failures"@) == u64::MAX { Some(queue_task::Error::Counter) } else { None },
            Change::Failed(event) => if queue_task::retryable(*event) && queue_task::count(before, "failures"@) == u64::MAX { Some(queue_task::Error::Counter) } else { None },
            Change::Progress { .. } => if crate::request::field_or_null(before, "spec"@) is Object { None } else { Some(queue_task::Error::Task) },
            _ => None,
        } }
    }
    pub open spec fn retry_fields(before: Value, after: Value) -> bool {
        queue_task::count(after, "failures"@) as int == queue_task::count(before, "failures"@) as int + 1
            && queue_task::status(after, if queue_task::count(before, "failures"@) as int + 1
                < queue_task::count(crate::request::field_or_null(before, "spec"@), "max_attempts"@) { "pending"@ } else { "failed"@ })
            && crate::json_specs::field(after, "lease"@) == Some(Value::Null)
    }
    pub open spec fn leaf_step(before: Value, after: Value, change: Change) -> bool {
        fault(before, change).is_none() && match change {
            Change::Retry => retry_fields(before, after)
                && queue_task::unchanged(before, after, set!["failures"@, "state"@, "lease"@]),
            Change::Lease(lease) => queue_task::status(after, "leased"@) && queue_task::copied(after, "lease"@, *lease)
                && queue_task::copied(after, "generation"@, crate::request::field_or_null(*lease, "generation"@))
                && queue_task::unchanged(before, after, set!["state"@, "generation"@, "lease"@]),
            Change::Failed(event) => crate::json_specs::field(after, "failure"@).is_some()
                && queue_task::failure_value(crate::request::field_or_null(after, "failure"@), *event)
                && if queue_task::retryable(*event) { retry_fields(before, after)
                    && queue_task::unchanged(before, after, set!["state"@, "failures"@, "lease"@, "failure"@]) }
                    else { queue_task::status(after, "failed"@) && queue_task::unchanged(before, after, set!["state"@, "failure"@]) },
            Change::Terminal(status) => queue_task::status(after, queue_task::terminal_name(status))
                && queue_task::unchanged(before, after, set!["state"@]),
            Change::Progress { event, yielding, order } => queue_task::revised_spec(crate::request::field_or_null(before, "spec"@),
                    crate::request::field_or_null(after, "spec"@), *event)
                && queue_task::unchanged(before, after, if yielding { set!["spec"@, "state"@, "lease"@, "pending_order"@] } else { set!["spec"@] })
                && (yielding ==> queue_task::status(after, "pending"@) && crate::json_specs::field(after, "lease"@) == Some(Value::Null)
                    && queue_task::unsigned(after, "pending_order"@, order)),
        }
    }
    pub open spec fn leaf_result(before: Value, after: Value, change: Change, result: Result<(), queue_task::Error>) -> bool {
        match result { Ok(()) => leaf_step(before, after, change), Err(error) => fault(before, change) == Some(error) && after == before }
    }
    pub open spec fn task_frame(before: Value, after: Value, id: String) -> bool {
        queue_task::unchanged(before, after, set!["tasks"@]) && tasks(before) is Object && tasks(after) is Object
            && table(after).contains_key(id) && table(after) == table(before).insert(id, table(after)[id])
    }
    pub open spec fn edit_result(before: Value, after: Value, id: String, change: Change, result: Result<(), Error>) -> bool {
        match result {
            Err(Error::Target) => !target(before, id) && after == before,
            Err(Error::Task(error)) => target(before, id) && after == before && fault(table(before)[id], change) == Some(error),
            Err(Error::Identity(_)) => false,
            Ok(()) => target(before, id) && task_frame(before, after, id) && leaf_step(table(before)[id], table(after)[id], change),
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == field_id(*value, name@).is_some(),
        match result { Ok(id) => field_id(*value, name@) == Some(*id), Err(error) => error == Error::Identity(name) },
))]
pub fn identity<'a>(value: &'a Value, name: &'static str) -> Result<&'a String, Error> {
    match name.index_into(value) {
        Some(Value::String(id)) => Ok(id),
        _ => Err(Error::Identity(name)),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures leaf_result(*old(task), *final(task), change, result),
))]
fn dispatch(task: &mut Value, change: Change<'_>) -> Result<(), queue_task::Error> {
    match change {
        Change::Retry => queue_task::retry(task),
        Change::Lease(lease) => queue_task::leased(task, lease),
        Change::Failed(event) => queue_task::failed(task, event),
        Change::Terminal(status) => queue_task::terminal(task, status),
        Change::Progress {
            event,
            yielding,
            order,
        } => queue_task::progress(task, event, yielding, order),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures edit_result(*old(state), *final(state), *id, change, result),
))]
#[allow(clippy::ptr_arg)]
pub fn edit(state: &mut Value, id: &String, change: Change<'_>) -> Result<(), Error> {
    #[cfg(straw_verus)]
    proof! { broadcast use vstd::std_specs::btree::group_btree_axioms; }
    let Some(value) = "tasks".index_into_mut(state) else {
        return Err(Error::Target);
    };
    let Value::Object(table) = value else {
        return Err(Error::Target);
    };
    let Some(task) = table.get_mut(id) else {
        return Err(Error::Target);
    };
    match dispatch(task, change) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Task(error)),
    }
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn submission_result(before: Value, after: Value, spec: Value, order: u64, result: Result<(), Error>) -> bool {
        (result == Err(Error::Identity("task_id")) <==> field_id(spec, "task_id"@).is_none())
            && (field_id(spec, "task_id"@).is_some() ==> (result.is_ok() == (tasks(before) is Object)
                && (result == Err(Error::Target) <==> !(tasks(before) is Object))))
            && (result.is_err() ==> after == before)
            && (result.is_ok() ==> task_frame(before, after, field_id(spec, "task_id"@)->Some_0)
                && queue_task::copied(table(after)[field_id(spec, "task_id"@)->Some_0], "spec"@, spec)
                && queue_task::status(table(after)[field_id(spec, "task_id"@)->Some_0], "pending"@)
                && queue_task::unsigned(table(after)[field_id(spec, "task_id"@)->Some_0], "generation"@, 0)
                && queue_task::unsigned(table(after)[field_id(spec, "task_id"@)->Some_0], "failures"@, 0)
                && queue_task::unsigned(table(after)[field_id(spec, "task_id"@)->Some_0], "pending_order"@, order)
                && crate::json_specs::field(table(after)[field_id(spec, "task_id"@)->Some_0], "lease"@) == Some(Value::Null))
            && (result.is_ok() ==> forall|name: Seq<char>| !set!["spec"@, "state"@, "generation"@, "failures"@, "lease"@, "pending_order"@].contains(name)
                ==> crate::json_specs::field(table(after)[field_id(spec, "task_id"@)->Some_0], name).is_none())
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures submission_result(*old(state), *final(state), *spec, order, result),
))]
pub fn submit(state: &mut Value, spec: &Value, order: u64) -> Result<(), Error> {
    let id = identity(spec, "task_id")?;
    let Some(value) = "tasks".index_into_mut(state) else {
        return Err(Error::Target);
    };
    let Value::Object(table) = value else {
        return Err(Error::Target);
    };
    table.insert(id.clone(), queue_task::submitted(spec, order));
    Ok(())
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn value_id(value: Value) -> Option<String> {
        match value { Value::String(id) => Some(id), _ => None }
    }
    pub open spec fn trace(before: Value, after: Value, ids: Seq<Value>, change: Change, states: Seq<Value>) -> bool {
        states.len() == ids.len() + 1 && states[0] == before && states.last() == after
            && forall|i: int| 0 <= i < ids.len() ==> value_id(ids[i]).is_some()
                && edit_result(states[i], states[i + 1], value_id(ids[i])->Some_0, change, Ok(()))
    }
    pub closed spec fn applied(before: Value, after: Value, ids: Seq<Value>, change: Change) -> bool {
        exists|states: Seq<Value>| trace(before, after, ids, change, states)
    }
    pub proof fn applied_empty(state: Value, change: Change)
        ensures applied(state, state, Seq::empty(), change),
    {
        reveal(applied);
        assert(trace(state, state, Seq::empty(), change, seq![state]));
    }
    pub proof fn applied_push(before: Value, middle: Value, after: Value, ids: Seq<Value>, change: Change, next: Value)
        requires applied(before, middle, ids, change), value_id(next).is_some(),
            edit_result(middle, after, value_id(next)->Some_0, change, Ok(())),
        ensures applied(before, after, ids.push(next), change),
    {
        reveal(applied);
        let states = choose|states: Seq<Value>| trace(before, middle, ids, change, states);
        assert forall|i: int| 0 <= i < ids.push(next).len() implies value_id(ids.push(next)[i]).is_some()
            && edit_result(states.push(after)[i], states.push(after)[i + 1], value_id(ids.push(next)[i])->Some_0, change, Ok(())) by {
            if i < ids.len() {
                assert(edit_result(states[i], states[i + 1], value_id(ids[i])->Some_0, change, Ok(())));
            } else { assert(states[i] == middle); }
        }
        assert(trace(before, after, ids.push(next), change, states.push(after)));
    }
    pub open spec fn failed_at(state: Value, id: Value, change: Change, error: Error) -> bool {
        match value_id(id) {
            None => error == Error::Identity("task ID"),
            Some(name) => edit_result(state, state, name, change, Err(error)),
        }
    }
    pub open spec fn many_result(before: Value, after: Value, ids: Seq<Value>, change: Change, result: Result<(), Error>) -> bool {
        match result {
            Ok(()) => applied(before, after, ids, change),
            Err(error) => exists|count: nat| count < ids.len()
                && applied(before, after, ids.take(count as int), change)
                && failed_at(after, ids[count as int], change, error),
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures many_result(*old(state), *final(state), ids@, change, result),
))]
pub fn edit_many(state: &mut Value, ids: &[Value], change: Change<'_>) -> Result<(), Error> {
    let mut index = 0usize;
    #[cfg(straw_verus)]
    proof! { assert(ids@.take(0) =~= Seq::<Value>::empty()); applied_empty(*state, change); }
    #[cfg_attr(straw_verus, verus_spec(
        invariant index <= ids.len(), applied(*old(state), *state, ids@.take(index as int), change),
            crate::environment::string_order_contract(),
        decreases ids.len() - index,
    ))]
    while index < ids.len() {
        let Value::String(id) = &ids[index] else {
            #[cfg(straw_verus)]
            proof! { assert(exists|count: nat| count == index && count < ids.len()
            && applied(*old(state), *state, ids@.take(count as int), change)
            && failed_at(*state, ids@[count as int], change, Error::Identity("task ID"))); }
            return Err(Error::Identity("task ID"));
        };
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(*state));
        if let Err(error) = edit(state, id, change) {
            #[cfg(straw_verus)]
            proof! { assert(exists|count: nat| count == index && count < ids.len()
            && applied(*old(state), *state, ids@.take(count as int), change)
            && failed_at(*state, ids@[count as int], change, error)); }
            return Err(error);
        }
        #[cfg(straw_verus)]
        proof! {
            applied_push(*old(state), before@, *state, ids@.take(index as int), change, ids@[index as int]);
            assert(ids@.take(index as int + 1) == ids@.take(index as int).push(ids@[index as int]));
        }
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! { assert(ids@.take(index as int) == ids@); }
    Ok(())
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == value_id(*value).is_some(),
        match result { Ok(id) => value_id(*value) == Some(*id), Err(error) => error == Error::Identity("task ID") },
))]
pub fn task_id(value: &Value) -> Result<&String, Error> {
    match value {
        Value::String(id) => Ok(id),
        _ => Err(Error::Identity("task ID")),
    }
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn selected(state: Value, id: String) -> Option<Value> {
        if table(state).contains_key(id) { Some(table(state)[id]) } else { None }
    }
    pub proof fn history_frame(before: Value, after: Value, ids: Seq<Value>, change: Change, peer: String, field: Seq<char>)
        requires applied(before, after, ids, change), field != "tasks"@,
            forall|i: int| 0 <= i < ids.len() ==> value_id(ids[i]) != Some(peer),
        ensures crate::json_specs::field(after, field) == crate::json_specs::field(before, field),
            selected(after, peer) == selected(before, peer),
    {
        reveal(applied);
        let states = choose|states: Seq<Value>| trace(before, after, ids, change, states);
        trace_frame(before, after, ids, change, states, peer, field, ids.len());
    }
    pub proof fn trace_frame(before: Value, after: Value, ids: Seq<Value>, change: Change, states: Seq<Value>, peer: String, field: Seq<char>, count: nat)
        requires trace(before, after, ids, change, states), count <= ids.len(), field != "tasks"@,
            forall|i: int| 0 <= i < ids.len() ==> value_id(ids[i]) != Some(peer),
        ensures crate::json_specs::field(states[count as int], field) == crate::json_specs::field(before, field),
            selected(states[count as int], peer) == selected(before, peer),
        decreases count,
    {
        if count > 0 {
            trace_frame(before, after, ids, change, states, peer, field, (count - 1) as nat);
            let at = count as int - 1;
            let name = value_id(ids[at])->Some_0;
            assert(name != peer);
            assert(edit_result(states[at], states[at + 1], name, change, Ok(())));
            assert(task_frame(states[at], states[at + 1], name));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn batch_failure_preserves_the_exact_prefix_and_stops() {
        let mut state = json!({"tasks":{
            "first":{"spec":{"max_attempts":4},"state":"leased","failures":0,"lease":{"attempt_id":"first"}},
            "exhausted":{"spec":{"max_attempts":4},"state":"leased","failures":u64::MAX,"lease":{"attempt_id":"second"}},
            "last":{"spec":{"max_attempts":4},"state":"leased","failures":0,"lease":{"attempt_id":"last"}}},
            "readers":{"protected":{"roots":[1,2,3]}},"checkpoints":{"preserved":true}});
        let before = state.clone();
        assert!(matches!(
            edit_many(
                &mut state,
                &[
                    json!("first"),
                    json!("first"),
                    json!("exhausted"),
                    json!("last")
                ],
                Change::Retry
            ),
            Err(Error::Task(queue_task::Error::Counter))
        ));
        assert_eq!(state["tasks"]["first"]["failures"], 2);
        assert_eq!(state["tasks"]["first"]["state"], "pending");
        assert!(state["tasks"]["first"]["lease"].is_null());
        for key in ["exhausted", "last"] {
            assert_eq!(state["tasks"][key], before["tasks"][key]);
        }
        for key in ["readers", "checkpoints"] {
            assert_eq!(state[key], before[key]);
        }
        let prior = state.clone();
        assert!(matches!(
            edit(&mut state, &"missing".to_owned(), Change::Retry),
            Err(Error::Target)
        ));
        assert_eq!(state, prior);
    }

    #[test]
    fn task_name_equal_to_table_name_does_not_modify_other_tables() {
        let mut state = json!({"tasks":{"peer":{"state":"pending"}},"readers":{"kept":[1,2]},"requests":{"kept":true}});
        let before = state.clone();
        let spec = json!({"task_id":"tasks","max_attempts":3,"metadata":{"kept":[4]}});
        submit(&mut state, &spec, 7).unwrap();
        edit(
            &mut state,
            &"tasks".to_owned(),
            Change::Terminal(Terminal::Cancelled),
        )
        .unwrap();
        assert_eq!(state["tasks"]["tasks"]["spec"], spec);
        assert_eq!(state["tasks"]["tasks"]["pending_order"], 7);
        assert_eq!(state["tasks"]["tasks"]["state"], "cancelled");
        assert_eq!(state["tasks"]["peer"], before["tasks"]["peer"]);
        assert_eq!(state["readers"], before["readers"]);
        assert_eq!(state["requests"], before["requests"]);
    }
}
