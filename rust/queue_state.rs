//! Bind task transitions to their actual queue table and execute ordered replay.
use crate::queue_task::{self, Terminal};
use serde_json::Value;
use serde_json::value::Index;

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

#[derive(Debug)]
pub enum Error {
    Identity(&'static str),
    Target,
    Task(queue_task::Error),
}

pub fn identity<'a>(value: &'a Value, name: &'static str) -> Result<&'a String, Error> {
    match name.index_into(value) {
        Some(Value::String(id)) => Ok(id),
        _ => Err(Error::Identity(name)),
    }
}

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

#[allow(clippy::ptr_arg)]
pub fn edit(state: &mut Value, id: &String, change: Change<'_>) -> Result<(), Error> {
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

pub fn edit_many(state: &mut Value, ids: &[Value], change: Change<'_>) -> Result<(), Error> {
    let mut index = 0usize;

    while index < ids.len() {
        let Value::String(id) = &ids[index] else {
            return Err(Error::Identity("task ID"));
        };

        edit(state, id, change)?;

        index += 1;
    }

    Ok(())
}

pub fn task_id(value: &Value) -> Result<&String, Error> {
    match value {
        Value::String(id) => Ok(id),
        _ => Err(Error::Identity("task ID")),
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
