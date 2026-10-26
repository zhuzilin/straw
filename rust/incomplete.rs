//! Roll back an unpublished write before reusing/sealing its writer.
//! The descriptor (including the exact saved prefix) survives failed cleanup.

pub fn writer_process(owner: u32, current: u32) -> bool {
    owner == current
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Truncate,
    SyncFile,
    UnlinkPartial,
    SyncParent,
    SealFinal,
}

pub struct State<T> {
    pub value: T,
    pub completed: u8,
    pub failed: bool,
    pub standalone: bool,
}

pub fn start<T>(value: T, standalone: bool) -> State<T> {
    State {
        value,
        completed: 0,
        failed: false,
        standalone,
    }
}

// A newly allocated private name may be registered or created even when setup
// reports an error. Until offset capture succeeds, retire the whole new name.

pub fn preparing<T>(value: T) -> State<(T, u64)> {
    start((value, 0), true)
}

pub fn capture_prefix<T>(state: &mut State<(T, u64)>, prefix: u64, standalone: bool) {
    state.value.1 = prefix;
    state.standalone = standalone;
}

pub fn restart<T>(state: &mut State<T>) {
    state.completed = 0;
    state.failed = false;
}

pub fn next<T>(state: &State<T>) -> Option<Effect> {
    if state.failed {
        return None;
    }
    match state.completed {
        0 if state.standalone => Some(Effect::UnlinkPartial),
        0 => Some(Effect::Truncate),
        1 if state.standalone => Some(Effect::SyncParent),
        1 => Some(Effect::SyncFile),
        2 if state.standalone => Some(Effect::SealFinal),
        _ => None,
    }
}

pub fn advance<T>(state: &mut State<T>, success: bool) {
    let limit = if state.standalone { 3 } else { 2 };
    if !state.failed && state.completed < limit {
        state.completed += if success { 1 } else { 0 };
        state.failed = !success;
    }
}

pub fn finished<T>(state: &State<T>) -> bool {
    !state.failed && state.completed == if state.standalone { 3 } else { 2 }
}
