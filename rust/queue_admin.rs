//! Ordered submission and scheduling-counter effects in actual queue replay.

use crate::{queue_reader, queue_receipt, queue_state, queue_transition};
use serde_json::Value;
use serde_json::value::Index;

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

pub fn has_capacity(order: usize, count: usize) -> bool {
    order.checked_add(count).is_some()
}

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

// The explicit error branch carries the erased completed-prefix witness.
#[allow(clippy::question_mark)]
pub fn submit_many(state: &mut Value, order: &mut usize, specs: &[Value]) -> Result<(), Error> {
    let mut index = 0usize;

    while index < specs.len() {
        if let Err(error) = submit(state, order, &specs[index]) {
            return Err(error);
        }

        index += 1;
    }

    Ok(())
}

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

pub(crate) fn array<'a>(event: &'a Value, name: &'static str) -> Result<&'a [Value], Error> {
    match name.index_into(event) {
        Some(Value::Array(items)) => Ok(items),
        _ => Err(Error::Array(name)),
    }
}

fn continue_producer(state: &mut Value, event: &Value) -> Result<(), Error> {
    let Some(producers) = "producers".index_into_mut(state) else {
        return Err(Error::State);
    };
    match queue_receipt::continue_producer(producers, event) {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Receipt(error)),
    }
}

pub fn submitted_event(state: &mut Value, order: &mut usize, event: &Value) -> Result<(), Error> {
    let specs = array(event, "tasks")?;
    submit_many(state, order, specs)?;

    continue_producer(state, event)
}

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

pub fn remember(state: &mut Value, event: &Value) -> Result<(), Error> {
    let Some(Value::Object(requests)) = "requests".index_into_mut(state) else {
        return Err(Error::State);
    };
    crate::request::remember(requests, event);
    Ok(())
}

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
