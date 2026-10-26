//! Actual task-record construction and replay state transitions.
use serde_json::value::Index;
use serde_json::{Map, Value};

#[derive(Debug)]
pub enum Error {
    Task,
    Counter,
}

#[derive(Clone, Copy)]
pub enum Terminal {
    Failed,
    Cancelled,
    Completed,
}

pub fn valid_target(kind: &str, task: &Value) -> bool {
    kind == "Submitted" || matches!(task, Value::Object(_))
}

fn number(value: &Value, name: &str) -> u64 {
    name.index_into(value).and_then(Value::as_u64).unwrap_or(0)
}

pub fn submitted(spec: &Value, order: u64) -> Value {
    let mut fields = Map::new();
    fields.insert("spec".to_owned(), spec.clone());
    fields.insert("state".to_owned(), Value::String("pending".to_owned()));
    fields.insert("generation".to_owned(), Value::Number(0u64.into()));
    fields.insert("failures".to_owned(), Value::Number(0u64.into()));
    fields.insert("lease".to_owned(), Value::Null);
    fields.insert("pending_order".to_owned(), Value::Number(order.into()));
    Value::Object(fields)
}

pub fn retry(task: &mut Value) -> Result<(), Error> {
    let failures = number(task, "failures");
    let limit = match "spec".index_into(task) {
        Some(spec) => number(spec, "max_attempts"),
        None => 0,
    };
    let Value::Object(fields) = task else {
        return Err(Error::Task);
    };
    let Some(failures) = crate::bounds::add(failures, 1) else {
        return Err(Error::Counter);
    };
    fields.insert("failures".to_owned(), Value::Number(failures.into()));
    let state = if failures < limit {
        "pending"
    } else {
        "failed"
    };
    fields.insert("state".to_owned(), Value::String(state.to_owned()));
    fields.insert("lease".to_owned(), Value::Null);
    Ok(())
}

pub fn terminal(task: &mut Value, state: Terminal) -> Result<(), Error> {
    let Value::Object(fields) = task else {
        return Err(Error::Task);
    };
    let name = match state {
        Terminal::Failed => "failed",
        Terminal::Cancelled => "cancelled",
        Terminal::Completed => "completed",
    };
    fields.insert("state".to_owned(), Value::String(name.to_owned()));
    Ok(())
}

pub fn leased(task: &mut Value, lease: &Value) -> Result<(), Error> {
    let Value::Object(fields) = task else {
        return Err(Error::Task);
    };
    fields.insert("state".to_owned(), Value::String("leased".to_owned()));
    let generation = match "generation".index_into(lease) {
        Some(value) => value.clone(),
        None => Value::Null,
    };
    fields.insert("generation".to_owned(), generation);
    fields.insert("lease".to_owned(), lease.clone());
    Ok(())
}

fn failure(event: &Value) -> Value {
    if let Some(value) = "failure".index_into(event) {
        return value.clone();
    }
    let reason = match "type".index_into(event) {
        Some(value) => value.clone(),
        None => Value::Null,
    };
    let mut fields = Map::new();
    fields.insert("reason".to_owned(), reason);
    Value::Object(fields)
}

pub fn failed(task: &mut Value, event: &Value) -> Result<(), Error> {
    let retryable = "retryable"
        .index_into(event)
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if retryable {
        retry(task)?;
    } else {
        terminal(task, Terminal::Failed)?;
    }
    let value = failure(event);
    let Value::Object(fields) = task else {
        return Err(Error::Task);
    };
    fields.insert("failure".to_owned(), value);
    Ok(())
}

fn copy_present(fields: &mut Map<String, Value>, event: &Value, name: &str) {
    if let Some(value) = name.index_into(event) {
        fields.insert(name.to_owned(), value.clone());
    }
}

pub fn progress(task: &mut Value, event: &Value, yielding: bool, order: u64) -> Result<(), Error> {
    {
        let Some(spec) = "spec".index_into_mut(task) else {
            return Err(Error::Task);
        };
        let Value::Object(fields) = spec else {
            return Err(Error::Task);
        };
        let input = match "input_ref".index_into(event) {
            Some(value) => value.clone(),
            None => Value::Null,
        };
        fields.insert("input_ref".to_owned(), input);
        copy_present(fields, event, "priority");
        copy_present(fields, event, "scheduling_key");
        copy_present(fields, event, "metadata");
    }
    if yielding {
        let Value::Object(fields) = task else {
            return Err(Error::Task);
        };
        fields.insert("state".to_owned(), Value::String("pending".to_owned()));
        fields.insert("lease".to_owned(), Value::Null);
        fields.insert("pending_order".to_owned(), Value::Number(order.into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exhausted_retry_does_not_wrap_or_modify_the_task() {
        let mut task = json!({"spec":{"max_attempts":u64::MAX}, "failures":u64::MAX,
            "generation":7,"state":"leased","lease":{"attempt_id":"attempt"}});
        let before = task.clone();
        assert!(matches!(retry(&mut task), Err(Error::Counter)));
        assert_eq!(task, before);
        assert!(matches!(
            failed(&mut task, &json!({"type":"Expired"})),
            Err(Error::Counter)
        ));
        assert_eq!(task, before);
    }
}
