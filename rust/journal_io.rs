//! Queue WAL I/O on the actual borrowed file, including every returned error.
use crate::file_adapter::{Library, NativeFile};

use crate::journal_state::State;
use crate::wal_recovery::{Boundary, Failure};
use crate::wal_sink::{Appender, Hooks};
use crate::wal_source::{Barrier, Source};
use serde_json::Value;
use std::fs::File;

pub(crate) fn recover<'a, L: Library, S: Barrier<NativeFile<'a, L>>>(
    file: &'a mut File,
    state: &mut State,
    sync: S,
) -> Result<Boundary, Failure<L::Error, crate::journal_recovery::Error>> {
    let file = NativeFile::<L>::new(file);
    let mut source = match Source::from_file(file, sync) {
        Ok(source) => source,
        Err(error) => return Err(Failure::Device(error)),
    };
    let result = state.recover(&mut source);
    source.finish();
    result
}

pub(crate) fn append<'a, L: Library, H: Hooks<NativeFile<'a, L>>>(
    file: &'a mut File,
    state: &mut State,
    events: &Value,
    hooks: H,
) -> Result<u64, crate::journal_append::Error<L::Error>> {
    let file = NativeFile::<L>::new(file);
    let mut writer = Appender::from_file(file, crate::framing::Kind::Queue, hooks);
    let result = crate::journal_append::append(state, &mut writer, events);
    writer.finish();
    result
}

/// Persist and apply retention before returning the selected GC roots.
pub(crate) fn retire<'a, L: Library, H: Hooks<NativeFile<'a, L>>>(
    file: &'a mut File,
    state: &mut State,
    business: &mut Value,
    roots: crate::queue_roots::Roots,
    hooks: H,
) -> Result<crate::queue_gc::Retired, crate::queue_retirement::Error<L::Error>> {
    let file = NativeFile::<L>::new(file);
    let mut writer = Appender::from_file(file, crate::framing::Kind::Queue, hooks);
    let result = crate::queue_retirement::commit(state, business, &mut writer, roots);
    writer.finish();
    result
}
