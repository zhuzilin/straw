//! Actual task-record construction and replay state transitions.
use serde_json::value::Index;
use serde_json::{Map, Value};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error {
    Task,
    Counter,
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy)]
pub enum Terminal {
    Failed,
    Cancelled,
    Completed,
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn count(value: Value, name: Seq<char>) -> u64 {
        match crate::json_specs::json(crate::request::field_or_null(value, name)) {
            crate::json_specs::Json::Number(Some(n)) => n,
            _ => 0,
        }
    }
    pub open spec fn unsigned(value: Value, name: Seq<char>, number: u64) -> bool {
        crate::json_specs::field(value, name).is_some()
            && crate::json_specs::json(crate::request::field_or_null(value, name))
                == crate::json_specs::Json::Number(Some(number))
    }
    pub open spec fn unchanged(before: Value, after: Value, changed: Set<Seq<char>>) -> bool {
        after is Object && forall|name: Seq<char>| !changed.contains(name)
            ==> crate::json_specs::field(after, name) == crate::json_specs::field(before, name)
    }
    pub open spec fn status(value: Value, state: Seq<char>) -> bool {
        crate::queue_event::text(value, "state"@) == Some(state)
    }
    pub open spec fn terminal_name(state: Terminal) -> Seq<char> {
        match state { Terminal::Failed => "failed"@, Terminal::Cancelled => "cancelled"@, Terminal::Completed => "completed"@ }
    }
    pub open spec fn copied(value: Value, name: Seq<char>, original: Value) -> bool {
        crate::json_specs::field(value, name).is_some()
            && crate::queue_metadata::same_value(crate::request::field_or_null(value, name), original)
    }
    pub open spec fn failure_value(value: Value, event: Value) -> bool {
        match crate::json_specs::field(event, "failure"@) {
            Some(original) => crate::queue_metadata::same_value(value, original),
            None => value is Object && copied(value, "reason"@, crate::request::field_or_null(event, "type"@))
                && forall|name: Seq<char>| name != "reason"@ ==> crate::json_specs::field(value, name).is_none(),
        }
    }
    pub open spec fn retryable(event: Value) -> bool {
        match crate::json_specs::json(crate::request::field_or_null(event, "retryable"@)) {
            crate::json_specs::Json::Boolean(b) => b, _ => true,
        }
    }
    pub open spec fn revised_spec(before: Value, after: Value, event: Value) -> bool {
        after is Object && copied(after, "input_ref"@, crate::request::field_or_null(event, "input_ref"@))
            && (forall|name: Seq<char>| set!["priority"@, "scheduling_key"@, "metadata"@].contains(name) ==>
                match crate::json_specs::field(event, name) {
                    Some(value) => copied(after, name, value),
                    None => crate::json_specs::field(after, name) == crate::json_specs::field(before, name),
                })
            && unchanged(before, after, set!["input_ref"@, "priority"@, "scheduling_key"@, "metadata"@])
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (kind@ == "Submitted"@ || *task is Object),
))]
pub fn valid_target(kind: &str, task: &Value) -> bool {
    kind == "Submitted" || matches!(task, Value::Object(_))
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == count(*value, name@)))]
fn number(value: &Value, name: &str) -> u64 {
    name.index_into(value).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures status(result, "pending"@), copied(result, "spec"@, *spec),
        unsigned(result, "generation"@, 0), unsigned(result, "failures"@, 0),
        unsigned(result, "pending_order"@, order),
        crate::json_specs::field(result, "lease"@) == Some(Value::Null),
        result is Object,
        forall|name: Seq<char>| !set!["spec"@, "state"@, "generation"@, "failures"@, "lease"@, "pending_order"@].contains(name)
            ==> crate::json_specs::field(result, name).is_none(),
))]
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

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures result == Err(Error::Task) <==> !(*old(task) is Object),
        result == Err(Error::Counter) <==> *old(task) is Object && count(*old(task), "failures"@) == u64::MAX,
        result.is_ok() <==> *old(task) is Object && count(*old(task), "failures"@) < u64::MAX,
        result.is_err() ==> *final(task) == *old(task),
        result.is_ok() ==> count(*final(task), "failures"@) as int == count(*old(task), "failures"@) as int + 1
            && status(*final(task), if count(*old(task), "failures"@) as int + 1
                < count(crate::request::field_or_null(*old(task), "spec"@), "max_attempts"@) { "pending"@ } else { "failed"@ })
            && crate::json_specs::field(*final(task), "lease"@) == Some(Value::Null)
            && unchanged(*old(task), *final(task), set!["failures"@, "state"@, "lease"@]),
))]
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

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures result.is_ok() == (*old(task) is Object),
        result.is_err() ==> result == Err(Error::Task) && *final(task) == *old(task),
        result.is_ok() ==> status(*final(task), terminal_name(state))
            && unchanged(*old(task), *final(task), set!["state"@]),
))]
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

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures result.is_ok() == (*old(task) is Object),
        result.is_err() ==> result == Err(Error::Task) && *final(task) == *old(task),
        result.is_ok() ==> status(*final(task), "leased"@) && copied(*final(task), "lease"@, *lease)
            && copied(*final(task), "generation"@, crate::request::field_or_null(*lease, "generation"@))
            && unchanged(*old(task), *final(task), set!["state"@, "generation"@, "lease"@]),
))]
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

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures failure_value(result, *event),
))]
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

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures result == Err(Error::Task) <==> !(*old(task) is Object),
        result == Err(Error::Counter) <==> *old(task) is Object && retryable(*event) && count(*old(task), "failures"@) == u64::MAX,
        result.is_ok() <==> *old(task) is Object && (!retryable(*event) || count(*old(task), "failures"@) < u64::MAX),
        result.is_err() ==> *final(task) == *old(task),
        result.is_ok() ==> crate::json_specs::field(*final(task), "failure"@).is_some()
            && failure_value(crate::request::field_or_null(*final(task), "failure"@), *event)
            && unchanged(*old(task), *final(task), if retryable(*event) {
                set!["state"@, "failures"@, "lease"@, "failure"@] } else { set!["state"@, "failure"@] })
            && if retryable(*event) {
                count(*final(task), "failures"@) as int == count(*old(task), "failures"@) as int + 1
                    && status(*final(task), if count(*old(task), "failures"@) as int + 1
                        < count(crate::request::field_or_null(*old(task), "spec"@), "max_attempts"@) { "pending"@ } else { "failed"@ })
                    && crate::json_specs::field(*final(task), "lease"@) == Some(Value::Null)
            } else { status(*final(task), "failed"@) },
))]
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

#[cfg_attr(straw_verus, verus_spec(
    requires crate::environment::string_order_contract(),
    ensures match crate::json_specs::field(*event, name@) {
        None => *final(fields) == *old(fields),
        Some(value) => copied(Value::Object(*final(fields)), name@, value)
            && unchanged(Value::Object(*old(fields)), Value::Object(*final(fields)), set![name@]),
    },
))]
fn copy_present(fields: &mut Map<String, Value>, event: &Value, name: &str) {
    if let Some(value) = name.index_into(event) {
        fields.insert(name.to_owned(), value.clone());
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures result.is_ok() <==> crate::request::field_or_null(*old(task), "spec"@) is Object,
        result.is_err() ==> result == Err(Error::Task) && *final(task) == *old(task),
        result.is_ok() ==> revised_spec(crate::request::field_or_null(*old(task), "spec"@),
                crate::request::field_or_null(*final(task), "spec"@), *event)
            && unchanged(*old(task), *final(task), if yielding { set!["spec"@, "state"@, "lease"@, "pending_order"@] } else { set!["spec"@] })
            && (yielding ==> status(*final(task), "pending"@)
                && crate::json_specs::field(*final(task), "lease"@) == Some(Value::Null)
                && unsigned(*final(task), "pending_order"@, order)),
))]
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
