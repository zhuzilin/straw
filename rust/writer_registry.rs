//! Writer registry JSON decoding. Successful events retain the exact field
//! values and their frame order; the journal authenticates/parses raw frames.
use serde_json::Value;

pub enum Event<'a> {
    Begin {
        sequence: u64,
        key: &'a str,
        owner: Option<&'a str>,
        dependencies: &'a Vec<Value>,
    },
    Results {
        operation: u64,
        roots: &'a Vec<Value>,
        intermediates: Option<&'a Vec<Value>>,
    },
    Done {
        operation: u64,
    },
    DoneOwner {
        owner: &'a str,
    },
    DoneKey {
        key: &'a str,
    },
    Retired {
        operations: Vec<u64>,
        results: &'a Vec<Value>,
    },
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    crate::catalog_json::get(value, key)?.as_str()
}

fn integer(value: &Value, key: &str) -> Option<u64> {
    crate::catalog_json::get(value, key)?.as_u64()
}

fn values<'a>(value: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    crate::catalog_json::get(value, key)?.as_array()
}

pub fn decode(value: &Value) -> Option<Event<'_>> {
    let kind = string(value, "type")?;
    if kind == "Begin" {
        let sequence = integer(value, "sequence")?;
        let key = string(value, "key")?;
        let v = crate::catalog_json::get(value, "owner")?;
        let owner = if v.is_null() { None } else { Some(v.as_str()?) };
        Some(Event::Begin {
            sequence,
            key,
            owner,
            dependencies: values(value, "dependencies")?,
        })
    } else if kind == "Results" {
        let operation = integer(value, "operation")?;
        let roots = values(value, "results")?;
        let intermediates = match crate::catalog_json::get(value, "intermediates") {
            None => None,
            Some(v) => Some(v.as_array()?),
        };
        Some(Event::Results {
            operation,
            roots,
            intermediates,
        })
    } else if kind == "Done" {
        Some(Event::Done {
            operation: integer(value, "operation")?,
        })
    } else if kind == "DoneOwner" {
        Some(Event::DoneOwner {
            owner: string(value, "owner")?,
        })
    } else if kind == "DoneKey" {
        Some(Event::DoneKey {
            key: string(value, "key")?,
        })
    } else if kind == "Retired" {
        let operations = values(value, "operations")?;
        let results = values(value, "results")?;
        let mut decoded = Vec::new();
        let mut index = 0;

        while index < operations.len() {
            let number = match operations[index].as_u64() {
                Some(number) => number,
                None => {
                    return None;
                }
            };
            decoded.push(number);
            index += 1;
        }
        Some(Event::Retired {
            operations: decoded,
            results,
        })
    } else {
        None
    }
}

pub fn decode_frame(value: &Value) -> Option<Vec<Event<'_>>> {
    let values = value.as_array()?;
    let mut result = Vec::new();
    let mut index = 0;

    while index < values.len() {
        let event = match decode(&values[index]) {
            Some(event) => event,
            None => {
                return None;
            }
        };
        result.push(event);
        index += 1;
    }
    Some(result)
}

pub type Evidence = crate::writer_retirement::Evidence<Vec<Value>>;

#[derive(Clone, PartialEq)]
pub struct Results {
    pub roots: Vec<Value>,
    pub intermediates: Vec<Value>,
}
pub type Operation = crate::writer_retirement::Operation<Evidence, Results>;

fn copy_values(values: &[Value]) -> Vec<Value> {
    let mut result = Vec::with_capacity(values.len());
    let mut index = 0;

    while index < values.len() {
        result.push(values[index].clone());
        index += 1;
    }
    result
}

pub fn begin(
    operations: &mut Vec<Operation>,
    frame: u64,
    sequence: u64,
    key: &str,
    owner: Option<&str>,
    dependencies: &[Value],
) -> bool {
    if frame != sequence {
        return false;
    }
    let owner = owner.map(|value| value.to_owned());
    let evidence = Evidence {
        key: key.to_owned(),
        owner,
        dependencies: copy_values(dependencies),
    };
    crate::writer_retirement::register_operation(operations, frame, evidence)
}

fn equal_values(old_values: &[Value], incoming: &[Value]) -> bool {
    if old_values.len() != incoming.len() {
        return false;
    }
    let mut index = 0;

    while index < old_values.len() {
        if old_values[index] != incoming[index] {
            return false;
        }
        index += 1;
    }
    true
}

pub fn describe(
    operations: &mut [Operation],
    sequence: u64,
    roots: &[Value],
    intermediates: &[Value],
) -> bool {
    let index = match crate::writer_retirement::operation_position(operations, sequence) {
        Some(index) => index,
        None => return false,
    };
    if operations[index].done {
        return false;
    }
    if let Some(previous) = &operations[index].results {
        return equal_values(&previous.roots, roots)
            && equal_values(&previous.intermediates, intermediates);
    }
    let results = Results {
        roots: copy_values(roots),
        intermediates: copy_values(intermediates),
    };
    crate::writer_retirement::remember_results(&mut operations[index], results, true)
}

pub fn settle_ids(operations: &mut [Operation], ids: &[u64]) -> bool {
    let mut index = 0;

    while index < ids.len() {
        if crate::writer_retirement::operation_position(operations, ids[index]).is_none() {
            return false;
        }
        index += 1;
    }

    index = 0;

    while index < operations.len() {
        if ids.contains(&operations[index].sequence) {
            crate::writer_retirement::settle_operation(&mut operations[index]);
        }
        index += 1;
    }
    true
}

fn contains_value(values: &[Value], value: &Value) -> bool {
    let mut index = 0;

    while index < values.len() {
        if values[index] == *value {
            return true;
        }
        index += 1;
    }
    false
}

fn keep_one(kept: &mut Vec<Value>, value: &Value) {
    if !contains_value(kept, value) {
        kept.push(value.clone());
    }
}

pub fn keep_results(kept: &mut Vec<Value>, incoming: &[Value]) {
    let mut index = 0;

    while index < incoming.len() {
        keep_one(kept, &incoming[index]);

        index += 1;
    }
}

pub fn apply_event(
    operations: &mut Vec<Operation>,
    receipts: &mut Vec<Value>,
    frame: u64,
    event: &Event<'_>,
) -> bool {
    match event {
        Event::Begin {
            sequence,
            key,
            owner,
            dependencies,
        } => begin(operations, frame, *sequence, key, *owner, dependencies),
        Event::Results {
            operation,
            roots,
            intermediates,
        } => {
            let intermediate_refs: &[Value] = match intermediates {
                None => &[],
                Some(values) => values.as_slice(),
            };
            describe(operations, *operation, roots, intermediate_refs)
        }
        Event::Done { operation } => {
            let ids = [*operation];

            settle_ids(operations, &ids)
        }
        Event::DoneOwner { owner } => {
            crate::writer_retirement::settle_named(operations, owner, true);
            true
        }
        Event::DoneKey { key } => {
            crate::writer_retirement::settle_named(operations, key, false);
            true
        }
        Event::Retired {
            operations: ids,
            results,
        } => {
            if !settle_ids(operations, ids) {
                return false;
            }
            keep_results(receipts, results);
            true
        }
    }
}

pub fn apply_frame(
    operations: &mut Vec<Operation>,
    receipts: &mut Vec<Value>,
    frame: u64,
    value: &Value,
) -> bool {
    let events = match decode_frame(value) {
        Some(events) => events,
        None => return false,
    };

    let mut index = 0;

    while index < events.len() {
        if !apply_event(operations, receipts, frame, &events[index]) {
            return false;
        }

        index += 1;
    }

    true
}

pub fn replay(
    operations: &mut Vec<Operation>,
    receipts: &mut Vec<Value>,
    frames: &[Value],
) -> bool {
    if frames.is_empty() {
        return false;
    }

    let mut frame = 1;

    while frame < frames.len() {
        if !apply_frame(operations, receipts, frame as u64, &frames[frame]) {
            return false;
        }

        frame += 1;
    }

    true
}

/// Settlement is deliberately irrelevant: Done records the internal handoff,
/// not delivery of the reply to the caller. An unfinished later attempt also
/// cannot hide a result already recorded for the same exact API/request key.
pub fn receipt<'a>(operations: &'a [Operation], key: &str) -> Option<&'a Vec<Value>> {
    let mut index = operations.len();

    while index > 0 {
        index -= 1;
        if operations[index].evidence.key.as_str() == key
            && let Some(results) = &operations[index].results
        {
            return Some(&results.roots);
        }
    }
    None
}

/// Each leaf is one named native action; selection of the exact registered
/// roots, key, owner and settlement IDs remains in the checked implementation.
/// An error may have completed that leaf, so retry starts from fresh replay.
pub trait RecoveryIo {
    type Error;

    fn validate(&mut self, roots: &[Value]) -> Result<(), Self::Error>;

    fn stage(&mut self, roots: &[Value]) -> Result<(), Self::Error>;

    fn consume(&mut self, key: &str, refs: &[Value]) -> Result<(), Self::Error>;

    fn release(&mut self, owner: &str) -> Result<(), Self::Error>;

    fn commit(&mut self, ids: &[u64], roots: &[Value]) -> Result<(), Self::Error>;

    fn checkpoint(&mut self) -> Result<(), Self::Error>;
}

fn recover_operation<I: RecoveryIo>(
    io: &mut I,
    operation: &Operation,
    recover: bool,
    receipts: &mut Vec<Value>,
) -> Result<(), I::Error> {
    if recover && let Some(results) = &operation.results {
        io.validate(&results.roots)?;
        io.stage(&results.roots)?;
        keep_results(receipts, &results.roots);
    }
    Ok(())
}

fn release_operation<I: RecoveryIo>(io: &mut I, operation: &Operation) -> Result<(), I::Error> {
    if let Some(results) = &operation.results
        && !results.intermediates.is_empty()
    {
        io.consume(&operation.evidence.key, &results.intermediates)?;
    }
    if let Some(owner) = &operation.evidence.owner {
        io.release(owner)?;
    }
    io.checkpoint()
}

pub fn reconcile_registered<I: RecoveryIo>(
    io: &mut I,
    operations: &[Operation],
    recover: bool,
    receipts: &mut Vec<Value>,
) -> Result<(), I::Error> {
    let mut index = 0;

    while index < operations.len() {
        recover_operation(io, &operations[index], recover, receipts)?;

        index += 1;
    }

    index = 0;

    while index < operations.len() {
        release_operation(io, &operations[index])?;

        index += 1;
    }

    if !operations.is_empty() {
        let mut ids = Vec::new();
        index = 0;

        while index < operations.len() {
            ids.push(operations[index].sequence);
            index += 1;
        }

        io.commit(&ids, receipts)?;
    }
    Ok(())
}
