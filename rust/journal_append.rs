//! Actual queue event validation, encoding and durable append composition.
use crate::journal_state::State;
use crate::wal_device::Writer;
use serde_json::Value;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub enum Error<E> {
    Unavailable,
    Events,
    Json(serde_json::Error),
    Write(crate::journal_state::AppendFailure<E>),
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn encoded(events: Value, raw: Seq<u8>) -> bool {
        raw.len() <= usize::MAX && crate::journal_recovery::events(events) && crate::json_specs::serializes(&events, raw)
            && crate::json_specs::parses(raw, events)
    }
    pub(crate) open spec fn accepted(before: State, after: State, events: Value, bytes: Seq<u8>) -> bool {
        exists|raw: Seq<u8>| encoded(events, raw)
            && crate::wal_recovery::complete_frame(crate::framing::Kind::Queue,
                bytes, before.offset as int, before.sequence as nat, raw, after.offset as int)
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == Err(Error::Events) <==> !crate::journal_recovery::events(*events),
        result.is_ok() ==> encoded(*events, result->Ok_0@),
        result.is_err() ==> result->Err_0 is Events || result->Err_0 is Json,
))]
pub fn encode<E>(events: &Value) -> Result<Vec<u8>, Error<E>> {
    if !crate::journal_recovery::valid_events(events) {
        return Err(Error::Events);
    }
    let payload = match serde_json::to_vec(events) {
        Ok(payload) => payload,
        Err(error) => return Err(Error::Json(error)),
    };
    #[cfg(straw_verus)]
    proof! { vstd::std_specs::vec::axiom_spec_len(&payload); crate::json_specs::value_encoding(*events, payload@); }
    Ok(payload)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(writer).valid(), !old(writer)@.buffered,
    ensures final(writer).loan() == old(writer).loan(), final(writer).valid(), !final(writer)@.buffered, final(writer)@.identity == old(writer)@.identity,
        final(log).read_only == old(log).read_only,
        final(log).transactions@ == old(log).transactions@,
        final(log).incomplete_tail == old(log).incomplete_tail,
        result == Err(Error::Unavailable) <==> old(log).poisoned || old(log).read_only,
        result.is_err() && !(result->Err_0 is Write) ==> *final(log) == *old(log) && final(writer)@ == old(writer)@,
        result == Err(Error::Events) ==> !crate::journal_recovery::events(*events),
        result.is_err() ==> final(log).offset == old(log).offset && final(log).sequence == old(log).sequence,
        result.is_err() && result->Err_0 is Write ==> final(log).poisoned && crate::journal_recovery::events(*events),
        result.is_ok() ==> !final(log).poisoned && !final(log).read_only
            && result->Ok_0 == old(log).sequence && final(log).sequence as int == old(log).sequence as int + 1
            && final(log).offset == final(writer)@.cursor && final(log).offset == final(writer)@.bytes.len()
            && final(log).offset == final(writer)@.durable
            && final(writer)@.bytes.take(old(writer)@.bytes.len() as int) == old(writer)@.bytes
            && accepted(*old(log), *final(log), *events, final(writer)@.bytes),
))]
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
    #[cfg(straw_verus)]
    proof! {
        assert(exists|raw: Seq<u8>| raw == payload@ && encoded(*events, raw)
            && crate::wal_recovery::complete_frame(crate::framing::Kind::Queue,
                writer@.bytes, old(log).offset as int, old(log).sequence as nat, raw, log.offset as int));
    }
    Ok(sequence)
}
