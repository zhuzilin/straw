//! Actual catalog run binding, serialization and append/history composition.
use crate::catalog_decode::{self, Resolver};
use crate::catalog_log::LogState;
use crate::catalog_state::Operation;
use serde_json::Value;
use std::collections::HashMap;

pub struct Prepared {
    pub operations: Vec<Operation>,
    pub payload: Vec<u8>,
}

pub enum Error<R, W> {
    Decode(catalog_decode::Failure<R>),
    Json(serde_json::Error),
    Append(crate::catalog_log::AppendFailure<W>),
}

pub fn prepare<R: Resolver, W>(
    resolver: &mut R,
    run: &str,
    event: Value,
    known: &HashMap<String, Vec<String>>,
) -> Result<Prepared, Error<R::Error, W>> {
    let Value::Object(mut object) = event else {
        return Err(Error::Decode(catalog_decode::Failure::Wire(
            crate::catalog_json::DecodeError::Object,
        )));
    };
    object.insert("run_id".to_owned(), Value::String(run.to_owned()));
    let value = Value::Object(object);

    let decoded = catalog_decode::decode(resolver, &value, known);
    let operations = match decoded {
        Ok(operations) => operations,
        Err(error) => {
            return Err(Error::Decode(error));
        }
    };
    let payload = match serde_json::to_vec(&value) {
        Ok(payload) => payload,
        Err(error) => return Err(Error::Json(error)),
    };

    Ok(Prepared {
        operations,
        payload,
    })
}

pub fn append<R: Resolver, D: crate::wal_device::Writer>(
    log: &mut LogState,
    resolver: &mut R,
    run: &str,
    event: Value,
    device: &mut D,
) -> Result<(), Error<R::Error, D::Error>> {
    let prepared = prepare(resolver, run, event, &log.indexes().staging_edges)?;

    let result = log.append_to(device, prepared.operations, &prepared.payload);

    match result {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Append(error)),
    }
}
