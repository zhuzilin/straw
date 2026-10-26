//! Reader protection records used by actual queue replay.
use serde_json::value::Index;
use serde_json::{Map, Value};

#[derive(Debug)]
pub enum Error {
    Identity,
    State,
}

pub fn identity(lease: &Value) -> Option<&String> {
    match "attempt_id".index_into(lease) {
        Some(Value::String(id)) => Some(id),
        _ => None,
    }
}

fn contains(items: &[Value], input: &Value) -> bool {
    let mut index = 0usize;

    while index < items.len() {
        if items[index] == *input {
            return true;
        }
        index += 1;
    }
    false
}

fn create(lease: &Value, input: &Value) -> Value {
    let mut entry = Map::new();
    entry.insert("lease".to_owned(), lease.clone());
    entry.insert("roots".to_owned(), Value::Array(vec![input.clone()]));
    Value::Object(entry)
}

fn extend(entry: &mut Value, input: &Value) -> Result<(), Error> {
    let Some(value) = "roots".index_into_mut(entry) else {
        return Err(Error::State);
    };
    let Value::Array(roots) = value else {
        return Err(Error::State);
    };

    if !contains(roots, input) {
        roots.push(input.clone());
    }

    Ok(())
}

pub fn register(readers: &mut Value, lease: &Value, input: &Value) -> Result<(), Error> {
    if input.is_null() {
        return Ok(());
    }
    let Some(id) = identity(lease) else {
        return Err(Error::Identity);
    };
    let Value::Object(table) = readers else {
        return Err(Error::State);
    };
    table.insert(id.clone(), create(lease, input));
    Ok(())
}

pub fn progress(readers: &mut Value, lease: &Value, input: &Value) -> Result<(), Error> {
    if input.is_null() {
        return Ok(());
    }
    let Some(id) = identity(lease) else {
        return Ok(());
    };
    let Value::Object(table) = readers else {
        return Err(Error::State);
    };
    match table.get_mut(id) {
        Some(entry) => {
            if entry.is_null() {
                *entry = create(lease, input);
            } else {
                extend(entry, input)?;
            }
        }
        None => {
            table.insert(id.clone(), create(lease, input));
        }
    }
    Ok(())
}

#[allow(clippy::ptr_arg)]
pub fn remove(readers: &mut Value, id: &String) -> Result<(), Error> {
    let Value::Object(table) = readers else {
        return Err(Error::State);
    };
    table.remove(id);
    Ok(())
}

pub fn done(readers: &mut Value, leases: &[Value]) -> Result<(), Error> {
    if !matches!(readers, Value::Object(_)) {
        return Err(Error::State);
    }
    let mut index = 0usize;

    while index < leases.len() {
        let Some(id) = identity(&leases[index]) else {
            return Err(Error::Identity);
        };
        remove(readers, id)?;
        index += 1;
    }
    Ok(())
}

pub fn matches_worker(entry: &Value, worker: &Value) -> bool {
    let lease = "lease".index_into(entry).unwrap_or(&Value::Null);
    let owner = "worker_id".index_into(lease).unwrap_or(&Value::Null);
    *owner == *worker
}

fn keep_worker(entry: &mut Value, worker: &Value) -> bool {
    !matches_worker(entry, worker)
}

// Use one callback body in both builds. The verifier-only expansion adds its
// contract; the native expansion is an ordinary stable Rust closure.

macro_rules! reader_filter {
    ($entry:ident, $worker:ident, $body:block) => {
        |_: &String, $entry: &mut Value| -> bool { $body }
    };
}

pub fn retire(readers: &mut Value, worker: &Value) -> Result<(), Error> {
    let Value::Object(table) = readers else {
        return Err(Error::State);
    };
    let predicate = reader_filter!(entry, worker, { keep_worker(entry, worker) });

    table.retain(predicate);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn progress_preserves_roots_and_uses_native_value_equality() {
        let lease = json!({"attempt_id":"a", "worker_id":"old"});
        let mut readers = json!({"peer":{"lease":{"worker_id":"peer"},"roots":["peer-root"]}});
        register(&mut readers, &lease, &json!(-0.0)).unwrap();
        let old = readers.clone();
        progress(
            &mut readers,
            &json!({"attempt_id":"a","worker_id":"new"}),
            &json!(0.0),
        )
        .unwrap();
        assert_eq!(readers, old);
        let input = json!({"nested":[9.901469416441159e-145_f64, {"x":7}]});
        progress(&mut readers, &lease, &input).unwrap();
        assert_eq!(readers["a"]["lease"], lease);
        assert_eq!(readers["a"]["roots"][0], json!(-0.0));
        assert_eq!(readers["a"]["roots"][1], input);
        assert_eq!(readers["peer"], old["peer"]);
        let before = readers.clone();
        progress(&mut readers, &lease, &input).unwrap();
        assert_eq!(readers, before);
        assert!(matches!(
            done(&mut readers, &[lease, json!({"attempt_id":false})]),
            Err(Error::Identity)
        ));
        assert_eq!(readers, json!({"peer":old["peer"]}));
    }
}
