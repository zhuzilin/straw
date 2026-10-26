//! Ordered task/reader/receipt effects in queue business replay.
use crate::{queue_reader, queue_receipt, queue_state, queue_task};
use serde_json::Value;
use serde_json::value::Index;

#[derive(Debug)]
pub enum Error {
    Task(queue_state::Error),
    Reader(queue_reader::Error),
    Receipt(queue_receipt::Error),
}

fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    key.index_into(value).unwrap_or(&Value::Null)
}

#[allow(clippy::ptr_arg)]
fn task<'a>(state: &'a Value, id: &String) -> &'a Value {
    let Value::Object(table) = get(state, "tasks") else {
        return &Value::Null;
    };
    table.get(id).unwrap_or(&Value::Null)
}

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

    match queue_state::edit(state, id, change) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Task(error)),
    }
}

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

    let result = match queue_receipt::record(state, receipt, event_lease) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Receipt(error)),
    };

    match result {
        Ok(()) => Ok(id),
        Err(error) => Err(error),
    }
}

#[allow(clippy::ptr_arg)]
fn register_current(state: &mut Value, id: &String, event_lease: &Value) -> Result<(), Error> {
    let input = get(get(task(state, id), "spec"), "input_ref");
    let input = input.clone();

    let Some(readers) = "readers".index_into_mut(state) else {
        return Err(Error::Reader(queue_reader::Error::State));
    };
    match queue_reader::register(readers, event_lease, &input) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Reader(error)),
    }
}

pub fn leased(state: &mut Value, event_lease: &Value) -> Result<(), Error> {
    let id = match queue_state::identity(event_lease, "task_id") {
        Ok(id) => id,
        Err(error) => return Err(Error::Task(error)),
    };
    if let Err(error) = queue_state::edit(state, id, queue_state::Change::Lease(event_lease)) {
        return Err(Error::Task(error));
    }

    register_current(state, id, event_lease)
}

#[allow(clippy::ptr_arg)]
pub fn failed(state: &mut Value, id: &String, event: &Value) -> Result<(), Error> {
    let release = get(event, "type").as_str() == Some("Failed");
    finish(state, id, queue_state::Change::Failed(event), release)
}

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

#[allow(clippy::ptr_arg)]
fn progress_current(state: &mut Value, id: &String, new_input: &Value) -> Result<(), Error> {
    let snapshot = get(task(state, id), "lease").clone();

    progress_readers(state, &snapshot, new_input)
}

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

    let result = match queue_state::edit(state, id, change) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Task(error)),
    };

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
