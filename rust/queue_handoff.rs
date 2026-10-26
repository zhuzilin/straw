//! Production handoff envelope and exact raw roundtrip correspondence.
use crate::closure_node::Digest;
use serde_json::Value;
use std::collections::HashMap;

pub fn envelope(prefix: &str, sequence: u64, business: &[Value]) -> Value {
    let mut items = Vec::with_capacity(business.len());
    let mut index = 0;

    while index < business.len() {
        items.push(business[index].clone());
        index += 1;
    }

    let parts = vec![
        Value::String(prefix.to_owned()),
        Value::Number(sequence.into()),
        Value::Array(items),
    ];
    Value::Array(parts)
}

#[derive(Debug)]
pub enum Error<E> {
    Transaction,
    Misplaced,
    Repeated,
    Identity,
    Empty,
    Digest(E),
}

pub fn marker(value: &Value) -> bool {
    crate::queue_roots::field_value(value, "type").as_str() == Some("StagingHandoff")
}

pub fn no_markers(events: &[Value]) -> bool {
    let mut i = 0;

    while i < events.len() {
        if marker(&events[i]) {
            return false;
        }
        i += 1;
    }
    true
}

pub fn identify<D: Digest>(
    digester: &mut D,
    prefix: &str,
    sequence: u64,
    business: &[Value],
) -> Result<String, D::Error> {
    let preimage = envelope(prefix, sequence, business);
    let id = digester.digest(&preimage)?;

    Ok(id)
}

pub fn marker_event(id: &str) -> Value {
    let mut object = serde_json::Map::new();
    object.insert(
        "type".to_owned(),
        Value::String("StagingHandoff".to_owned()),
    );
    object.insert("id".to_owned(), Value::String(id.to_owned()));
    Value::Object(object)
}

pub fn attach<D: Digest>(
    digester: &mut D,
    prefix: &str,
    sequence: u64,
    events: &mut Vec<Value>,
    references: Vec<Value>,
) -> Result<Option<(String, Vec<Value>)>, D::Error> {
    if references.is_empty() {
        return Ok(None);
    }
    let id = identify(digester, prefix, sequence, events)?;
    events.push(marker_event(&id));
    Ok(Some((id, references)))
}

pub fn values(roots: HashMap<String, Value>) -> Vec<Value> {
    let mut it = roots.into_iter();

    let mut result = Vec::new();

    loop {
        let Some((key, value)) = it.next() else {
            return result;
        };

        let _ = key;
        result.push(value);
    }
}

pub fn roots<D: Digest>(
    digester: &mut D,
    business: &[Value],
) -> Result<HashMap<String, Value>, D::Error> {
    let mut output = HashMap::new();
    let mut i = 0;

    while i < business.len() {
        crate::queue_roots::references(digester, &business[i], &mut output)?;

        i += 1;
    }

    Ok(output)
}

#[allow(
    clippy::type_complexity,
    reason = "Keep component error types visible at the checked I/O boundary."
)]
pub fn recover<D: Digest>(
    digester: &mut D,
    prefix: &str,
    sequence: u64,
    transaction: &Value,
) -> Result<Option<(String, Vec<Value>)>, Error<D::Error>> {
    let Some(events) = transaction.as_array() else {
        return Err(Error::Transaction);
    };
    if events.is_empty() || !marker(&events[events.len() - 1]) {
        if !no_markers(events) {
            return Err(Error::Misplaced);
        }
        return Ok(None);
    }
    let business = &events[..events.len() - 1];
    if !no_markers(business) {
        return Err(Error::Repeated);
    }

    let id = match identify(digester, prefix, sequence, business) {
        Ok(id) => id,
        Err(error) => {
            return Err(Error::Digest(error));
        }
    };
    if crate::queue_roots::field_value(&events[events.len() - 1], "id").as_str()
        != Some(id.as_str())
    {
        return Err(Error::Identity);
    }
    let found = match roots(digester, business) {
        Ok(found) => found,
        Err(error) => {
            return Err(Error::Digest(error));
        }
    };
    if found.is_empty() {
        return Err(Error::Empty);
    }

    let references = values(found);

    Ok(Some((id, references)))
}

pub fn recover_into<D: Digest>(
    digester: &mut D,
    prefix: &str,
    sequence: u64,
    transaction: &Value,
    pending: &mut Vec<(String, Vec<Value>)>,
) -> Result<(), Error<D::Error>> {
    let outcome = recover(digester, prefix, sequence, transaction)?;

    if let Some(value) = outcome {
        pending.push(value);
    }

    Ok(())
}
