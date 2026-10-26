//! Shared queue/catalog WAL effect ordering and frame-boundary decisions.
//! Hashes, decoded lengths and actual I/O outcomes are adapter inputs.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppendEffect {
    Header,
    Payload,
    Trailer,
    Sync,
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug)]
pub struct Append {
    pub completed: u8,
    pub failed: bool,
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result@ == crate::protocol::initial()))]
pub fn append_start() -> Append {
    Append {
        completed: 0,
        failed: false,
    }
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == append_effect(state@)))]
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

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result@ == crate::protocol::advance(state@, success, 4),
        state@.completed <= 4 ==> result@.completed <= 4,
))]
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

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (!state@.failed && state@.completed == 4),
))]
pub fn append_finished(state: &Append) -> bool {
    !state.failed && state.completed == 4
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameExtent {
    Malformed,
    Truncated,
    Complete { end: u64 },
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures match result {
        FrameExtent::Malformed => offset > size || payload as int + 96 > u64::MAX as int,
        FrameExtent::Truncated => offset <= size && payload as int + 96 <= u64::MAX as int
            && offset as int + payload as int + 96 > size as int,
        FrameExtent::Complete { end } => offset <= end <= size
            && payload as int + 96 <= u64::MAX as int
            && end as int == offset as int + payload as int + 96,
    },
))]
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

#[cfg(straw_verus)]
verus! {
    impl View for Append {
        type V = crate::protocol::ProgressView;
        open spec fn view(&self) -> Self::V {
            crate::protocol::ProgressView { completed: self.completed as nat, failed: self.failed }
        }
    }
    pub open spec fn append_effect(state: crate::protocol::ProgressView) -> Option<AppendEffect> {
        if state.failed { None }
        else if state.completed == 0 { Some(AppendEffect::Header) }
        else if state.completed == 1 { Some(AppendEffect::Payload) }
        else if state.completed == 2 { Some(AppendEffect::Trailer) }
        else if state.completed == 3 { Some(AppendEffect::Sync) }
        else { None }
    }
    pub proof fn append_effect_order(state: crate::protocol::ProgressView)
        ensures append_effect(state) == Some(AppendEffect::Sync) ==> state.completed == 3 && !state.failed,
            state.failed ==> append_effect(state).is_none(),
    {}
}
