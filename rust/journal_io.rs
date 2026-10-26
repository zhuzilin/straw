//! Queue WAL I/O on the actual borrowed file, including every returned error.
use crate::file_adapter::{Library, NativeFile};
#[cfg(straw_verus)]
use crate::file_primitives::{Permission, identity};
use crate::journal_state::State;
use crate::wal_recovery::{Boundary, Failure};
use crate::wal_sink::{Appender, Hooks};
use crate::wal_source::{Barrier, Source};
use serde_json::Value;
use std::fs::File;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(permission)@@.identity == identity(*file),
    ensures identity(*final(file)) == identity(*old(file)),
        final(permission)@@.identity == identity(*final(file)),
        final(state).read_only == old(state).read_only,
        result.is_err() ==> final(state).sequence == old(state).sequence
            && final(state).offset == old(state).offset
            && (*final(state) == *old(state) || final(state).poisoned),
        result.is_ok() ==> !final(state).poisoned
            && final(state).sequence == final(state).transactions@.len()
            && final(state).sequence == result->Ok_0.sequence
            && final(state).offset == result->Ok_0.end
            && final(state).offset == final(permission)@@.cursor
            && final(state).incomplete_tail == result->Ok_0.incomplete,
        result.is_ok() && !old(state).read_only ==> final(state).offset == final(permission)@@.bytes.len()
            && final(state).offset == final(permission)@@.durable,
        result.is_ok() ==> exists|raw: Seq<Seq<u8>>| crate::journal_recovery::decoded(raw, final(state).transactions@)
            && crate::wal_recovery::replayed(crate::framing::Kind::Queue, old(permission)@@.bytes,
                0, 0, Seq::empty(), raw)
            && final(state).offset == crate::wal_recovery::run_end(0, raw),
))]
pub(crate) fn recover<'a, L: Library, S: Barrier<NativeFile<'a, L>>>(
    file: &'a mut File,
    state: &mut State,
    sync: S,
    #[cfg(straw_verus)] permission: &'a mut Tracked<Permission>,
) -> Result<Boundary, Failure<L::Error, crate::journal_recovery::Error>> {
    let file = NativeFile::<L>::new(
        file,
        #[cfg(straw_verus)]
        permission,
    );
    let mut source = match Source::from_file(file, sync) {
        Ok(source) => source,
        Err(error) => return Err(Failure::Device(error)),
    };
    let result = state.recover(&mut source);
    source.finish();
    result
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(permission)@@.identity == identity(*file),
    ensures identity(*final(file)) == identity(*old(file)),
        final(permission)@@.identity == identity(*final(file)),
        final(state).read_only == old(state).read_only,
        final(state).transactions@ == old(state).transactions@,
        final(state).incomplete_tail == old(state).incomplete_tail,
        result == Err(crate::journal_append::Error::Unavailable) <==> old(state).poisoned || old(state).read_only,
        result.is_err() && !(result->Err_0 is Write) ==> *final(state) == *old(state)
            && final(permission)@@ == old(permission)@@,
        result == Err(crate::journal_append::Error::Events) ==> !crate::journal_recovery::events(*events),
        result.is_err() ==> final(state).offset == old(state).offset && final(state).sequence == old(state).sequence,
        result.is_err() && result->Err_0 is Write ==> final(state).poisoned && crate::journal_recovery::events(*events),
        result.is_ok() ==> !final(state).poisoned && !final(state).read_only
            && result->Ok_0 == old(state).sequence && final(state).sequence as int == old(state).sequence as int + 1
            && final(state).offset == final(permission)@@.cursor
            && final(state).offset == final(permission)@@.bytes.len()
            && final(state).offset == final(permission)@@.durable
            && final(permission)@@.bytes.take(old(permission)@@.bytes.len() as int) == old(permission)@@.bytes
            && crate::journal_append::accepted(*old(state), *final(state), *events, final(permission)@@.bytes),
))]
pub(crate) fn append<'a, L: Library, H: Hooks<NativeFile<'a, L>>>(
    file: &'a mut File,
    state: &mut State,
    events: &Value,
    hooks: H,
    #[cfg(straw_verus)] permission: &'a mut Tracked<Permission>,
) -> Result<u64, crate::journal_append::Error<L::Error>> {
    let file = NativeFile::<L>::new(
        file,
        #[cfg(straw_verus)]
        permission,
    );
    let mut writer = Appender::from_file(file, crate::framing::Kind::Queue, hooks);
    let result = crate::journal_append::append(state, &mut writer, events);
    writer.finish();
    result
}

/// Persist and apply retention before returning the selected GC roots.
#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(permission)@@.identity == identity(*file), roots.selected_for(*old(business)),
        crate::environment::string_order_contract(), crate::environment::string_content_contract(),
    ensures identity(*final(file)) == identity(*old(file)),
        final(permission)@@.identity == identity(*final(file)),
        result.is_ok() ==> result->Ok_0@ == roots && (result->Ok_0@).selected_for(*final(business))
            && !final(state).poisoned && final(permission)@@.durable == final(state).offset
            && exists|events:Value| crate::queue_retirement::transaction(events,roots.prefix,roots.positions@)
                && crate::journal_append::accepted(*old(state),*final(state),events,final(permission)@@.bytes)
                && crate::queue_metadata::outcome(*old(business),*final(business),crate::json_specs::json(events)->Array_0[0],Ok(())),
        match result { Err(crate::queue_retirement::Error::Before(_))
            | Err(crate::queue_retirement::Error::Apply(_))
            | Err(crate::queue_retirement::Error::After(_)) => final(state).poisoned, _ => true },
))]
pub(crate) fn retire<'a, L: Library, H: Hooks<NativeFile<'a, L>>>(
    file: &'a mut File,
    state: &mut State,
    business: &mut Value,
    roots: crate::queue_roots::Roots,
    hooks: H,
    #[cfg(straw_verus)] permission: &'a mut Tracked<Permission>,
) -> Result<crate::queue_gc::Retired, crate::queue_retirement::Error<L::Error>> {
    let file = NativeFile::<L>::new(
        file,
        #[cfg(straw_verus)]
        permission,
    );
    let mut writer = Appender::from_file(file, crate::framing::Kind::Queue, hooks);
    let result = crate::queue_retirement::commit(state, business, &mut writer, roots);
    writer.finish();
    result
}
