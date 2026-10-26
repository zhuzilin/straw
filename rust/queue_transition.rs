//! Ordered task/reader/receipt effects in queue business replay.
use crate::{queue_reader, queue_receipt, queue_state, queue_task};
use serde_json::Value;
use serde_json::value::Index;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error {
    Task(queue_state::Error),
    Reader(queue_reader::Error),
    Receipt(queue_receipt::Error),
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn field(value: Value, key: Seq<char>) -> Value { crate::request::field_or_null(value, key) }
    pub open spec fn readers(state: Value) -> Value { field(state, "readers"@) }
    pub open spec fn selected(state: Value, id: String) -> Value {
        if queue_state::target(state, id) { queue_state::table(state)[id] } else { Value::Null }
    }
    pub open spec fn lease(state: Value, id: String) -> Value { field(selected(state, id), "lease"@) }
    pub open spec fn input(state: Value, id: String) -> Value { field(field(selected(state, id), "spec"@), "input_ref"@) }
    pub open spec fn reader_frame(before: Value, after: Value) -> bool {
        queue_task::unchanged(before, after, set!["readers"@])
    }
    pub open spec fn release_result(before: Value, after: Value, id: String, result: Result<(), Error>) -> bool {
        match queue_reader::attempt(lease(before, id)) {
            None => result.is_ok() && after == before,
            Some(attempt) => match result {
                Err(Error::Reader(queue_reader::Error::State)) => !(readers(before) is Object) && after == before,
                Ok(()) => readers(before) is Object && readers(after) is Object && reader_frame(before, after)
                    && queue_reader::table(readers(after)) == queue_reader::table(readers(before)).remove(attempt),
                _ => false,
            }
        }
    }
    pub closed spec fn finish_tail(before: Value, middle: Value, after: Value, id: String, change: queue_state::Change, release: bool, result: Result<(), Error>) -> bool {
        (if release { release_result(before, middle, id, Ok(())) } else { middle == before })
            && match result {
                Ok(()) => queue_state::edit_result(middle, after, id, change, Ok(())),
                Err(Error::Task(error)) => queue_state::edit_result(middle, after, id, change, Err(error)),
                _ => false,
            }
    }
    pub open spec fn finish_result(before: Value, after: Value, id: String, change: queue_state::Change, release: bool, result: Result<(), Error>) -> bool {
        (exists|middle: Value| finish_tail(before, middle, after, id, change, release, result))
            || release && release_result(before, after, id, result) && result.is_err()
    }
    pub closed spec fn complete_tail(before: Value, middle: Value, after: Value, id: String, receipt: Value, event_lease: Value, result: Result<(), Error>) -> bool {
        finish_result(before, middle, id, queue_state::Change::Terminal(queue_task::Terminal::Completed), true, Ok(()))
            && match result {
                Ok(()) => queue_receipt::outcome(middle, after, receipt, event_lease, Ok(())),
                Err(Error::Receipt(error)) => queue_receipt::outcome(middle, after, receipt, event_lease, Err(error)),
                _ => false,
            }
    }
    pub open spec fn complete_result(before: Value, after: Value, receipt: Value, event_lease: Value, result: Result<(), Error>) -> bool {
        match queue_state::field_id(receipt, "task_id"@) {
            None => result == Err(Error::Task(queue_state::Error::Identity("task_id"))) && after == before,
            Some(id) => (exists|middle: Value| complete_tail(before, middle, after, id, receipt, event_lease, result)) || result.is_err() && finish_result(before, after, id,
                    queue_state::Change::Terminal(queue_task::Terminal::Completed), true, result),
        }
    }
    pub open spec fn register_result(before: Value, after: Value, id: String, lease: Value, result: Result<(), Error>) -> bool {
        if crate::json_specs::field(before, "readers"@).is_none() { result == Err(Error::Reader(queue_reader::Error::State)) && after == before }
        else if input(before, id) == Value::Null { result.is_ok() && after == before }
        else if queue_reader::attempt(lease).is_none() {
            result == Err(Error::Reader(queue_reader::Error::Identity)) && after == before
        } else { match result {
            Err(Error::Reader(queue_reader::Error::State)) => !(readers(before) is Object) && after == before,
            Ok(()) => readers(before) is Object && readers(after) is Object && reader_frame(before, after)
                && queue_reader::table(readers(after)).contains_key(queue_reader::attempt(lease)->Some_0)
                && queue_reader::table(readers(after)) == queue_reader::table(readers(before)).insert(
                    queue_reader::attempt(lease)->Some_0, queue_reader::table(readers(after))[queue_reader::attempt(lease)->Some_0])
                && queue_reader::created(queue_reader::table(readers(after))[queue_reader::attempt(lease)->Some_0], lease, input(before, id)),
            _ => false,
        } }
    }
    pub closed spec fn leased_tail(before: Value, middle: Value, after: Value, id: String, event_lease: Value, result: Result<(), Error>) -> bool {
        queue_state::edit_result(before, middle, id, queue_state::Change::Lease(&event_lease), Ok(()))
            && register_result(middle, after, id, event_lease, result)
    }
    pub open spec fn leased_result(before: Value, after: Value, event_lease: Value, result: Result<(), Error>) -> bool {
        let name = queue_state::field_id(event_lease, "task_id"@);
        if name.is_none() { result == Err(Error::Task(queue_state::Error::Identity("task_id"))) && after == before }
        else {
            let id = name->Some_0;
            (exists|middle: Value| leased_tail(before, middle, after, id, event_lease, result))
                || match result { Err(Error::Task(error)) => queue_state::edit_result(before, after, id, queue_state::Change::Lease(&event_lease), Err(error)), _ => false }
        }
    }

    pub proof fn leased_finish(before: Value, middle: Value, after: Value, id: String, event_lease: Value, result: Result<(), Error>)
        requires queue_state::field_id(event_lease, "task_id"@) == Some(id),
            leased_tail(before, middle, after, id, event_lease, result),
        ensures leased_result(before, after, event_lease, result),
    {
        assert(queue_state::field_id(event_lease, "task_id"@).is_some());
        assert(queue_state::field_id(event_lease, "task_id"@)->Some_0 == id);
        assert(exists|mid: Value| leased_tail(before, mid, after, id, event_lease, result));
        reveal(leased_result);
    }

    pub open spec fn unit<T>(result: Result<T, Error>) -> Result<(), Error> {
        match result { Ok(_) => Ok(()), Err(error) => Err(error) }
    }
    pub proof fn complete_finish(before: Value, middle: Value, after: Value, id: String, receipt: Value, event_lease: Value, result: Result<(), Error>)
        requires queue_state::field_id(receipt, "task_id"@) == Some(id),
            complete_tail(before, middle, after, id, receipt, event_lease, result),
        ensures complete_result(before, after, receipt, event_lease, result),
    {
        assert(queue_state::field_id(receipt, "task_id"@).is_some());
        assert(queue_state::field_id(receipt, "task_id"@)->Some_0 == id);
        assert(exists|mid: Value| complete_tail(before, mid, after, id, receipt, event_lease, result));
    }
    pub open spec fn cancelled_result(before: Value, after: Value, event: Value, result: Result<(), Error>) -> bool {
        match queue_state::field_id(event, "task_id"@) {
            None => result == Err(Error::Task(queue_state::Error::Identity("task_id"))) && after == before,
            Some(id) => finish_result(before, after, id, queue_state::Change::Terminal(queue_task::Terminal::Cancelled), false, result),
        }
    }
    pub proof fn no_release_preserves_readers(before: Value, after: Value, id: String, change: queue_state::Change, result: Result<(), Error>)
        requires finish_result(before, after, id, change, false, result),
        ensures readers(after) == readers(before),
    {
        let middle = choose|middle: Value| finish_tail(before, middle, after, id, change, false, result);
        reveal(finish_tail);
        assert(middle == before);
    }
    pub open spec fn reader_progress_result(before: Value, after: Value, event_lease: Value, new_input: Value, result: Result<(), Error>) -> bool {
        if crate::json_specs::field(before, "readers"@).is_none() { result == Err(Error::Reader(queue_reader::Error::State)) && after == before }
        else if new_input == Value::Null || queue_reader::attempt(event_lease).is_none() { result.is_ok() && after == before }
        else {
            let id = queue_reader::attempt(event_lease)->Some_0;
            let valid = readers(before) is Object && match queue_reader::selected(readers(before), id) {
                None | Some(Value::Null) => true, Some(entry) => queue_reader::progressable(entry),
            };
            match result {
                Err(Error::Reader(queue_reader::Error::State)) => !valid && after == before,
                Ok(()) => valid && readers(after) is Object && reader_frame(before, after)
                    && queue_reader::table(readers(after)).contains_key(id)
                    && queue_reader::table(readers(after)) == queue_reader::table(readers(before)).insert(id, queue_reader::table(readers(after))[id])
                    && queue_reader::new_or_extended(queue_reader::selected(readers(before), id), queue_reader::table(readers(after))[id], event_lease, new_input),
                _ => false,
            }
        }
    }
    pub closed spec fn current_progress(before: Value, after: Value, id: String, new_input: Value, result: Result<(), Error>) -> bool {
        exists|snapshot: Value| crate::queue_metadata::same_value(snapshot, lease(before, id))
            && reader_progress_result(before, after, snapshot, new_input, result)
    }
    pub proof fn progress_snapshot(before: Value, after: Value, id: String, new_input: Value, result: Result<(), Error>) -> (snapshot: Value)
        requires current_progress(before, after, id, new_input, result),
        ensures crate::queue_metadata::same_value(snapshot, lease(before, id))
            && reader_progress_result(before, after, snapshot, new_input, result),
    {
        reveal(current_progress);
        choose|snapshot: Value| crate::queue_metadata::same_value(snapshot, lease(before, id))
            && reader_progress_result(before, after, snapshot, new_input, result)
    }
    pub closed spec fn progress_tail(before: Value, middle: Value, after: Value, id: String, event: Value, order: u64, result: Result<(), Error>) -> bool {
        current_progress(before, middle, id, field(event, "input_ref"@), Ok(()))
            && match result {
                Ok(()) => queue_state::edit_result(middle, after, id, queue_state::Change::Progress { event: &event, yielding: false, order }, Ok(())),
                Err(Error::Task(error)) => queue_state::edit_result(middle, after, id, queue_state::Change::Progress { event: &event, yielding: false, order }, Err(error)),
                _ => false,
            }
    }
    pub open spec fn progress_result(before: Value, after: Value, event: Value, yielding: bool, order: u64, result: Result<(), Error>) -> bool {
        match queue_state::field_id(event, "task_id"@) {
            None => result == Err(Error::Task(queue_state::Error::Identity("task_id"))) && after == before,
            Some(id) => if yielding { finish_result(before, after, id, queue_state::Change::Progress { event: &event, yielding, order }, true, result) }
                else { (exists|middle: Value| progress_tail(before, middle, after, id, event, order, result))
                    || result.is_err() && current_progress(before, after, id, field(event, "input_ref"@), result) },
        }
    }
    pub proof fn progress_finish(before: Value, middle: Value, after: Value, id: String, event: Value, order: u64, result: Result<(), Error>)
        requires queue_state::field_id(event, "task_id"@) == Some(id), progress_tail(before, middle, after, id, event, order, result),
        ensures progress_result(before, after, event, false, order, result),
    {
        assert(queue_state::field_id(event, "task_id"@).is_some());
        assert(queue_state::field_id(event, "task_id"@)->Some_0 == id);
        assert(exists|mid: Value| progress_tail(before, mid, after, id, event, order, result));
    }
    pub proof fn progress_middle(before: Value, after: Value, event: Value, order: u64, id: String) -> (middle: Value)
        requires progress_result(before, after, event, false, order, Ok(())), queue_state::field_id(event, "task_id"@) == Some(id),
        ensures current_progress(before, middle, id, field(event, "input_ref"@), Ok(()))
            && queue_state::edit_result(middle, after, id, queue_state::Change::Progress { event: &event, yielding: false, order }, Ok(())),
    {
        let middle = choose|middle: Value| progress_tail(before, middle, after, id, event, order, Ok(()));
        reveal(progress_tail);
        middle
    }

}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures *result == field(*value, key@),
))]
fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    key.index_into(value).unwrap_or(&Value::Null)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures *result == selected(*state, *id),
))]
#[allow(clippy::ptr_arg)]
fn task<'a>(state: &'a Value, id: &String) -> &'a Value {
    #[cfg(straw_verus)]
    proof! { broadcast use vstd::std_specs::btree::group_btree_axioms; }
    let Value::Object(table) = get(state, "tasks") else {
        return &Value::Null;
    };
    table.get(id).unwrap_or(&Value::Null)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures release_result(*old(state), *final(state), *id, result),
))]
#[allow(clippy::ptr_arg)]
fn release_current(state: &mut Value, id: &String) -> Result<(), Error> {
    let Some(attempt) = queue_reader::identity(get(task(state, id), "lease")) else {
        return Ok(());
    };
    let attempt = attempt.clone();
    let Some(readers) = "readers".index_into_mut(state) else {
        return Err(Error::Reader(queue_reader::Error::State));
    };
    match queue_reader::remove(readers, &attempt) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Reader(error)),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures finish_result(*old(state), *final(state), *id, change, release, result),
))]
#[allow(clippy::ptr_arg)]
pub fn finish(
    state: &mut Value,
    id: &String,
    change: queue_state::Change<'_>,
    release: bool,
) -> Result<(), Error> {
    if release {
        release_current(state, id)?;
    }
    #[cfg(straw_verus)]
    let middle = verus_exec_expr!(Ghost(*state));
    #[cfg(straw_verus)]
    proof! { assert(if release { release_result(*old(state), middle@, *id, Ok(())) } else { middle@ == *old(state) }); }
    let result = match queue_state::edit(state, id, change) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Task(error)),
    };
    #[cfg(straw_verus)]
    proof! {
        assert(if release { release_result(*old(state), middle@, *id, Ok(())) } else { middle@ == *old(state) });
        assert(match result { Ok(()) => queue_state::edit_result(middle@, *state, *id, change, Ok(())), Err(Error::Task(error)) => queue_state::edit_result(middle@, *state, *id, change, Err(error)), _ => false });
        reveal(finish_tail);
        assert(finish_tail(*old(state), middle@, *state, *id, change, release, result));
    }
    result
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures complete_result(*old(state), *final(state), *receipt, *event_lease, unit(result)),
        result.is_ok() ==> queue_state::field_id(*receipt, "task_id"@) == Some(*result->Ok_0),
))]
pub fn complete<'a>(
    state: &mut Value,
    receipt: &'a Value,
    event_lease: &Value,
) -> Result<&'a String, Error> {
    let id = match queue_state::identity(receipt, "task_id") {
        Ok(id) => id,
        Err(error) => return Err(Error::Task(error)),
    };
    finish(
        state,
        id,
        queue_state::Change::Terminal(queue_task::Terminal::Completed),
        true,
    )?;
    #[cfg(straw_verus)]
    let middle = verus_exec_expr!(Ghost(*state));
    let result = match queue_receipt::record(state, receipt, event_lease) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Receipt(error)),
    };
    #[cfg(straw_verus)]
    proof! {
        reveal(complete_tail);
        assert(complete_tail(*old(state), middle@, *state, *id, *receipt, *event_lease, result));
        complete_finish(*old(state), middle@, *state, *id, *receipt, *event_lease, result);
    }
    match result {
        Ok(()) => Ok(id),
        Err(error) => Err(error),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures register_result(*old(state), *final(state), *id, *event_lease, result),
))]
#[allow(clippy::ptr_arg)]
fn register_current(state: &mut Value, id: &String, event_lease: &Value) -> Result<(), Error> {
    let input = get(get(task(state, id), "spec"), "input_ref");
    let input = input.clone();
    #[cfg(straw_verus)]
    proof! { assert(crate::json_specs::observe(input, 1) == crate::json_specs::observe(crate::queue_transition::input(*old(state), *id), 1)); }
    let Some(readers) = "readers".index_into_mut(state) else {
        return Err(Error::Reader(queue_reader::Error::State));
    };
    match queue_reader::register(readers, event_lease, &input) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Reader(error)),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures leased_result(*old(state), *final(state), *event_lease, result),
))]
pub fn leased(state: &mut Value, event_lease: &Value) -> Result<(), Error> {
    let id = match queue_state::identity(event_lease, "task_id") {
        Ok(id) => id,
        Err(error) => return Err(Error::Task(error)),
    };
    if let Err(error) = queue_state::edit(state, id, queue_state::Change::Lease(event_lease)) {
        return Err(Error::Task(error));
    }
    #[cfg(straw_verus)]
    let middle = verus_exec_expr!(Ghost(*state));
    let result = register_current(state, id, event_lease);
    #[cfg(straw_verus)]
    proof! {
        reveal(leased_tail);
        assert(leased_tail(*old(state), middle@, *state, *id, *event_lease, result));
        leased_finish(*old(state), middle@, *state, *id, *event_lease, result);
    }
    result
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures finish_result(*old(state), *final(state), *id, queue_state::Change::Failed(event),
        crate::json_specs::text(field(*event, "type"@)) == Some("Failed"@), result),
))]
#[allow(clippy::ptr_arg)]
pub fn failed(state: &mut Value, id: &String, event: &Value) -> Result<(), Error> {
    let release = get(event, "type").as_str() == Some("Failed");
    finish(state, id, queue_state::Change::Failed(event), release)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures cancelled_result(*old(state), *final(state), *event, unit(result)),
        result.is_ok() ==> queue_state::field_id(*event, "task_id"@) == Some(*result->Ok_0),
))]
pub fn cancelled<'a>(state: &mut Value, event: &'a Value) -> Result<&'a String, Error> {
    let id = match queue_state::identity(event, "task_id") {
        Ok(id) => id,
        Err(error) => return Err(Error::Task(error)),
    };
    finish(
        state,
        id,
        queue_state::Change::Terminal(queue_task::Terminal::Cancelled),
        false,
    )?;
    Ok(id)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures reader_progress_result(*old(state), *final(state), *event_lease, *new_input, result),
))]
fn progress_readers(
    state: &mut Value,
    event_lease: &Value,
    new_input: &Value,
) -> Result<(), Error> {
    let Some(readers) = "readers".index_into_mut(state) else {
        return Err(Error::Reader(queue_reader::Error::State));
    };
    match queue_reader::progress(readers, event_lease, new_input) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Reader(error)),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures current_progress(*old(state), *final(state), *id, *new_input, result),
))]
#[allow(clippy::ptr_arg)]
fn progress_current(state: &mut Value, id: &String, new_input: &Value) -> Result<(), Error> {
    let snapshot = get(task(state, id), "lease").clone();
    let result = progress_readers(state, &snapshot, new_input);
    #[cfg(straw_verus)]
    proof! {
        reveal(current_progress);
        assert(crate::queue_metadata::same_value(snapshot, lease(*old(state), *id)));
        assert(exists|copy: Value| copy == snapshot && crate::queue_metadata::same_value(copy, lease(*old(state), *id))
            && reader_progress_result(*old(state), *state, copy, *new_input, result));
    }
    result
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures progress_result(*old(state), *final(state), *event, yielding, order, unit(result)),
        result.is_ok() ==> queue_state::field_id(*event, "task_id"@) == Some(*result->Ok_0),
))]
pub fn progress<'a>(
    state: &mut Value,
    event: &'a Value,
    yielding: bool,
    order: u64,
) -> Result<&'a String, Error> {
    let id = match queue_state::identity(event, "task_id") {
        Ok(id) => id,
        Err(error) => return Err(Error::Task(error)),
    };
    let change = queue_state::Change::Progress {
        event,
        yielding,
        order,
    };
    if yielding {
        finish(state, id, change, true)?;
        return Ok(id);
    }
    progress_current(state, id, get(event, "input_ref"))?;
    #[cfg(straw_verus)]
    let middle = verus_exec_expr!(Ghost(*state));
    let result = match queue_state::edit(state, id, change) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Task(error)),
    };
    #[cfg(straw_verus)]
    proof! {
        reveal(progress_tail);
        assert(progress_tail(*old(state), middle@, *state, *id, *event, order, result));
        progress_finish(*old(state), middle@, *state, *id, *event, order, result);
    }
    match result {
        Ok(()) => Ok(id),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state() -> Value {
        json!({
            "tasks": {"task": {"state":"leased", "failures":0,
                "lease":{"attempt_id":"attempt", "worker_id":"worker"},
                "spec":{"max_attempts":3, "input_ref":"input"}}},
            "readers": {"attempt":{"lease":{"attempt_id":"attempt"}, "roots":["first", "second"]},
                "peer":{"lease":{"attempt_id":"peer"}, "roots":["peer-root"]}},
            "submissions":{}, "commits":[], "checkpoints":{"keep":"checkpoint"}
        })
    }

    #[test]
    fn completion_error_preserves_its_exact_reader_task_and_submission_prefix() {
        let mut value = state();
        value["commits"] = Value::Bool(false);
        let receipt =
            json!({"task_id":"task", "submission_id":"submission", "result_ref":"result"});
        let event_lease = json!({"attempt_id":"receipt-attempt"});
        let mut expected = value.clone();
        expected["readers"]
            .as_object_mut()
            .unwrap()
            .remove("attempt");
        expected["tasks"]["task"]["state"] = json!("completed");
        expected["submissions"]["submission"] = json!({"receipt":receipt,"lease":event_lease});
        assert!(matches!(
            complete(&mut value, &receipt, &event_lease),
            Err(Error::Receipt(queue_receipt::Error::State))
        ));
        assert_eq!(value, expected);
        let before = value.clone();
        assert!(complete(&mut value, &json!({"task_id":false}), &event_lease).is_err());
        assert_eq!(value, before);
    }

    #[test]
    fn lease_registration_error_keeps_the_completed_task_update() {
        let mut value = state();
        let event_lease = json!({"task_id":"task", "generation":9});
        let mut expected = value.clone();
        expected["tasks"]["task"]["generation"] = json!(9);
        expected["tasks"]["task"]["lease"] = event_lease.clone();
        assert!(matches!(
            leased(&mut value, &event_lease),
            Err(Error::Reader(queue_reader::Error::Identity))
        ));
        assert_eq!(value, expected);
        let before = value.clone();
        assert!(
            leased(
                &mut value,
                &json!({"task_id":"missing", "attempt_id":"attempt"})
            )
            .is_err()
        );
        assert_eq!(value, before);
    }

    #[test]
    fn expiration_and_cancellation_keep_roots_but_failure_releases_before_retry() {
        let id = "task".to_owned();
        for kind in ["Expired", "Failed"] {
            let mut value = state();
            value["tasks"]["task"]["failures"] = json!(u64::MAX);
            let mut expected = value.clone();
            if kind == "Failed" {
                expected["readers"]
                    .as_object_mut()
                    .unwrap()
                    .remove("attempt");
            }
            assert!(matches!(
                failed(&mut value, &id, &json!({"type":kind})),
                Err(Error::Task(queue_state::Error::Task(
                    queue_task::Error::Counter
                )))
            ));
            assert_eq!(value, expected);
        }
        let mut value = state();
        let before = value.clone();
        let event = json!({"task_id":"task"});
        assert_eq!(cancelled(&mut value, &event).unwrap(), &id);
        assert_eq!(value["readers"], before["readers"]);
        assert_eq!(value["tasks"]["task"]["state"], "cancelled");
    }

    #[test]
    fn progress_error_keeps_new_and_prior_roots_while_yield_releases_only_its_reader() {
        for yielding in [false, true] {
            let mut value = state();
            value["tasks"]["task"]["spec"] = Value::Bool(false);
            let mut expected = value.clone();
            if yielding {
                expected["readers"]
                    .as_object_mut()
                    .unwrap()
                    .remove("attempt");
            } else {
                expected["readers"]["attempt"]["roots"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("latest"));
            }
            let event = json!({"task_id":"task", "input_ref":"latest"});
            assert!(matches!(
                progress(&mut value, &event, yielding, 17),
                Err(Error::Task(queue_state::Error::Task(
                    queue_task::Error::Task
                )))
            ));
            assert_eq!(value, expected);
        }
        let mut value = state();
        let event = json!({"task_id":"task", "input_ref":"latest"});
        progress(&mut value, &event, false, 17).unwrap();
        assert_eq!(
            value["readers"]["attempt"]["roots"],
            json!(["first", "second", "latest"])
        );
        progress(&mut value, &event, true, 18).unwrap();
        assert_eq!(value["readers"].as_object().unwrap().len(), 1);
        assert_eq!(value["tasks"]["task"]["pending_order"], 18);
        assert_eq!(value["tasks"]["task"]["lease"], Value::Null);
    }
}
