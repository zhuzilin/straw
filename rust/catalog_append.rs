//! Actual catalog run binding, serialization and append/history composition.
use crate::catalog_decode::{self, Resolver};
use crate::catalog_log::LogState;
use crate::catalog_state::Operation;
use serde_json::Value;
use std::collections::HashMap;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub struct Prepared {
    pub operations: Vec<Operation>,
    pub payload: Vec<u8>,
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum Error<R, W> {
    Decode(catalog_decode::Failure<R>),
    Json(serde_json::Error),
    Append(crate::catalog_log::AppendFailure<W>),
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn binds_run(original: Value, encoded: Value, run: Seq<char>) -> bool {
        crate::json_specs::json(original) is Object
            && crate::json_specs::json(encoded) is Object
            && crate::catalog_frame::same_run(encoded, run)
            && forall|name: Seq<char>| name != "run_id"@
                ==> crate::json_specs::field(encoded, name) == crate::json_specs::field(original, name)
    }
    pub open spec fn intended(context: crate::catalog_frame::Context, original: Value,
        known: Map<String, Seq<String>>, raw: Seq<u8>, operations: Seq<Operation>) -> bool {
        exists|value: Value, wire: crate::catalog_json::Wire, keys: Map<String, Vec<String>>|
            binds_run(original, value, context.run) && crate::json_specs::parses(raw, value)
                && crate::catalog_json::schema(value) && crate::catalog_json::wire_matches(value, wire)
                && crate::catalog_state::edge_view(keys) == known && catalog_decode::admitted(wire, keys)
                && catalog_decode::resolved_wire(context.namespace, wire)
                && catalog_decode::meaning(context.namespace, wire, operations)
    }
    pub open spec fn decode_failure<E>(context: crate::catalog_frame::Context, original: Value,
        known: Map<String, Seq<String>>, error: catalog_decode::Failure<E>) -> bool {
        (crate::json_specs::json(original) !is Object
            && error == catalog_decode::Failure::Wire(crate::catalog_json::DecodeError::Object))
        || (exists|value: Value, keys: Map<String, Vec<String>>|
                binds_run(original, value, context.run) && crate::catalog_state::edge_view(keys) == known
                    && catalog_decode::decoded(context.namespace, value, keys, Err(error)))
    }
    pub proof fn failure_witness<E>(context: crate::catalog_frame::Context, original: Value,
        encoded: Value, known: Map<String, Vec<String>>, error: catalog_decode::Failure<E>)
        requires binds_run(original, encoded, context.run),
            catalog_decode::decoded(context.namespace, encoded, known, Err(error)),
        ensures decode_failure(context, original, crate::catalog_state::edge_view(known), error),
    {
        assert(exists|value: Value, keys: Map<String, Vec<String>>|
            binds_run(original, value, context.run)
                && crate::catalog_state::edge_view(keys) == crate::catalog_state::edge_view(known)
                && catalog_decode::decoded(context.namespace, value, keys, Err(error)));
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(resolver).valid(), crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
    ensures final(resolver).valid(), final(resolver)@ == old(resolver)@,
        match result {
            Err(Error::Decode(reason)) => decode_failure(
                crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ }, event,
                crate::catalog_state::edge_view(known@), reason),
            Err(Error::Append(_)) => false,
            _ => true,
        },
        result.is_ok() ==> intended(
            crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ }, event, crate::catalog_state::edge_view(known@),
            result->Ok_0.payload@, result->Ok_0.operations@),
        result.is_ok() ==> crate::catalog_frame::meaning(
            crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ },
            result->Ok_0.payload@, crate::catalog_state::edge_view(known@), result->Ok_0.operations@),
))]
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
    #[cfg(straw_verus)]
    proof! { assert(binds_run(event, value, run@)); }
    let decoded = catalog_decode::decode(resolver, &value, known);
    let operations = match decoded {
        Ok(operations) => operations,
        Err(error) => {
            #[cfg(straw_verus)]
            proof! {
                failure_witness(crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ },
                    event, value, known@, error);
            }
            return Err(Error::Decode(error));
        }
    };
    let payload = match serde_json::to_vec(&value) {
        Ok(payload) => payload,
        Err(error) => return Err(Error::Json(error)),
    };
    #[cfg(straw_verus)]
    proof! {
        crate::json_specs::value_encoding(value, payload@);
        assert(binds_run(event, value, run@));
        let wire = choose|wire: crate::catalog_json::Wire|
            crate::catalog_json::wire_matches(value, wire)
                && catalog_decode::construction_result(old(resolver)@, wire, known@, decoded);
        assert(crate::catalog_frame::meaning(
            crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ },
            payload@, crate::catalog_state::edge_view(known@), operations@));
        assert(intended(crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ },
            event, crate::catalog_state::edge_view(known@), payload@, operations@));
    }
    Ok(Prepared {
        operations,
        payload,
    })
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(resolver).valid(), crate::catalog_log::valid(old(log)@), old(device).valid(), !old(device)@.buffered,
        crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
        crate::catalog_target::interpreted(
            crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ }, old(log)@.history),
    ensures final(device).loan() == old(device).loan(), final(resolver).valid(), crate::catalog_log::valid(final(log)@), final(resolver)@ == old(resolver)@,
        match result {
            Err(Error::Decode(reason)) => decode_failure(
                crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ }, event,
                old(log)@.indexes.edges, reason),
            _ => true,
        },
        final(device).valid(), final(device)@.identity == old(device)@.identity, !final(device)@.buffered,
        crate::catalog_target::interpreted(
            crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ }, final(log)@.history),
        result.is_ok() ==> crate::catalog_log::ready(final(log)@)
            && final(log)@.history.len() == old(log)@.history.len() + 1
            && final(log)@.history.drop_last() == old(log)@.history
            && final(device)@.cursor == final(log)@.offset
            && final(device)@.bytes.len() == final(log)@.offset
            && final(device)@.durable == final(log)@.offset
            && final(log)@.floor == final(log)@.offset,
        result.is_ok() ==> intended(
            crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ }, event,
            old(log)@.indexes.edges, final(log)@.history.last().payload,
            final(log)@.history.last().operations),
        result.is_err() ==> final(log)@.history == old(log)@.history && final(log)@.indexes == old(log)@.indexes,
        result.is_err() && !(result->Err_0 is Append) ==> final(log)@ == old(log)@
            && final(device)@ == old(device)@,
        match result {
            Err(Error::Append(crate::catalog_log::AppendFailure::Write(_))) =>
                !crate::catalog_log::ready(final(log)@) && final(log)@.dirty,
            Err(Error::Append(crate::catalog_log::AppendFailure::State(_))) =>
                final(log)@ == old(log)@ && final(device)@ == old(device)@,
            _ => true,
        },
))]
pub fn append<R: Resolver, D: crate::wal_device::Writer>(
    log: &mut LogState,
    resolver: &mut R,
    run: &str,
    event: Value,
    device: &mut D,
) -> Result<(), Error<R::Error, D::Error>> {
    let prepared = prepare(resolver, run, event, &log.indexes().staging_edges)?;
    #[cfg(straw_verus)]
    let before = verus_exec_expr!(Ghost(log@));
    let result = log.append_to(device, prepared.operations, &prepared.payload);
    #[cfg(straw_verus)]
    proof! {
        if result.is_ok() {
            crate::catalog_log::complete_history(before@);
            crate::catalog_target::interpret_push(
                crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ },
                before@.history,
                crate::catalog_log::Record { operations: prepared.operations@, payload: prepared.payload@ });
            assert(log@.history.drop_last() == before@.history);
        }
    }
    match result {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Append(error)),
    }
}
