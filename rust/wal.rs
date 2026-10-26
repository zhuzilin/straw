//! Shared queue/catalog WAL effect ordering and frame-boundary decisions.
//! Hashes, decoded lengths and actual I/O outcomes are adapter inputs.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppendEffect {
    Header,
    Payload,
    Trailer,
    Sync,
}

#[derive(Clone, Copy, Debug)]
pub struct Append {
    pub completed: u8,
    pub failed: bool,
}

pub fn append_start() -> Append {
    Append {
        completed: 0,
        failed: false,
    }
}

pub fn append_next(state: &Append) -> Option<AppendEffect> {
    if state.failed {
        return None;
    }
    match state.completed {
        0 => Some(AppendEffect::Header),
        1 => Some(AppendEffect::Payload),
        2 => Some(AppendEffect::Trailer),
        3 => Some(AppendEffect::Sync),
        _ => None,
    }
}

pub fn append_advance(state: Append, success: bool) -> Append {
    if state.failed || state.completed >= 4 {
        return state;
    }
    if success {
        Append {
            completed: state.completed + 1,
            failed: false,
        }
    } else {
        Append {
            completed: state.completed,
            failed: true,
        }
    }
}

pub fn append_finished(state: &Append) -> bool {
    !state.failed && state.completed == 4
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameExtent {
    Malformed,
    Truncated,
    Complete { end: u64 },
}

pub fn frame_extent(offset: u64, size: u64, payload: u64) -> FrameExtent {
    if offset > size || payload > u64::MAX - 96 {
        return FrameExtent::Malformed;
    }
    let frame = payload + 96;
    if frame > size - offset {
        return FrameExtent::Truncated;
    }
    FrameExtent::Complete {
        end: offset + frame,
    }
}
