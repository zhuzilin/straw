//! The queue journal's actual JSON validation and replay application.
use crate::wal_device::{Device, Target};
use crate::wal_recovery::{Boundary, Failure};
use serde_json::Value;

fn valid_event(value: &Value) -> bool {
    let Some(kind) = crate::catalog_json::get(value, "type") else {
        return false;
    };
    kind.as_str().is_some()
}

pub fn valid_events(value: &Value) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    let mut index = 0;

    while index < items.len() {
        if !valid_event(&items[index]) {
            return false;
        }
        index += 1;
    }
    true
}

pub enum Error {
    Json(serde_json::Error),
    Events,
}

pub struct Apply {
    transactions: Vec<Value>,
}

impl Target for Apply {
    type Error = Error;

    fn apply(&mut self, payload: &[u8]) -> Result<(), Error> {
        let value = match serde_json::from_slice::<Value>(payload) {
            Ok(value) => value,
            Err(error) => return Err(Error::Json(error)),
        };
        if !valid_events(&value) {
            return Err(Error::Events);
        }
        self.transactions.push(value);

        Ok(())
    }
}

// Use the audited replace contract without adding a generic Default contract.
#[allow(clippy::mem_replace_with_default)]
pub fn recover<D: Device>(
    device: &mut D,
    transactions: &mut Vec<Value>,
    writable: bool,
) -> Result<Boundary, Failure<D::Error, Error>> {
    // Restart from the physical prefix, including after an earlier partial apply.
    transactions.clear();
    let mut target = Apply {
        transactions: std::mem::replace(transactions, Vec::new()),
    };
    let result = crate::wal_recovery::recover(
        device,
        &mut target,
        crate::framing::Kind::Queue,
        0,
        0,
        0,
        writable,
    );

    *transactions = target.transactions;
    result
}
