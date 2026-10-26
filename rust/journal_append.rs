//! Actual queue event validation, encoding and durable append composition.
use crate::journal_state::State;
use crate::wal_device::Writer;
use serde_json::Value;

pub enum Error<E> {
    Unavailable,
    Events,
    Json(serde_json::Error),
    Write(crate::journal_state::AppendFailure<E>),
}

pub fn encode<E>(events: &Value) -> Result<Vec<u8>, Error<E>> {
    if !crate::journal_recovery::valid_events(events) {
        return Err(Error::Events);
    }
    let payload = match serde_json::to_vec(events) {
        Ok(payload) => payload,
        Err(error) => return Err(Error::Json(error)),
    };

    Ok(payload)
}

pub(crate) fn append<W: Writer>(
    log: &mut State,
    writer: &mut W,
    events: &Value,
) -> Result<u64, Error<W::Error>> {
    if log.poisoned || log.read_only {
        return Err(Error::Unavailable);
    }
    let payload = encode(events)?;
    let sequence = match log.append_to(writer, &payload) {
        Ok(sequence) => sequence,
        Err(error) => return Err(Error::Write(error)),
    };

    Ok(sequence)
}
