//! The actual catalog indexes, replay cursor and confirmation state.
//! I/O establishes durability; this state accepts only matching boundaries.
use crate::catalog_state::{Indexes, Operation};
use crate::{replay, wal};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Unavailable,
    Truncated,
    Boundary,
    InvalidOperation(usize),
    Mismatch,
}

pub enum AppendFailure<E> {
    State(Failure),
    Write(crate::wal_append::Failure<E>),
}

pub struct Pending {
    operations: Vec<Operation>,
    end: u64,
    sequence: u64,
}

pub struct LogState {
    indexes: Indexes,
    offset: u64,
    sequence: u64,
    recovery: replay::Recovery,
    complete: bool,
    pending: Option<Pending>,
}

impl LogState {
    pub fn new() -> Self {
        Self {
            indexes: Indexes::new(),
            offset: 0,
            sequence: 0,
            recovery: replay::start(),
            complete: true,
            pending: None,
        }
    }

    pub fn indexes(&self) -> &Indexes {
        &self.indexes
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn floor(&self) -> u64 {
        self.recovery.durable
    }

    pub fn dirty(&self) -> bool {
        self.recovery.rebuild
    }

    pub fn ready(&self) -> bool {
        replay::ready(&self.recovery) && self.complete && self.pending.is_none()
    }

    pub fn invalidate(&mut self) {
        replay::prepare(&mut self.recovery, true);
    }

    /// Return whether derived namespace/closure caches must also be cleared.
    pub fn begin_replay(&mut self, size: u64) -> Result<bool, Failure> {
        if !replay::accepts(&self.recovery, size) || (!self.recovery.rebuild && size < self.offset)
        {
            return Err(Failure::Truncated);
        }
        let rebuilt = self.dirty();
        if rebuilt {
            self.indexes = Indexes::new();
            self.offset = 0;
            self.sequence = 0;
            self.complete = true;
            self.pending = None;
        }
        replay::prepare(&mut self.recovery, self.offset < size);
        Ok(rebuilt)
    }

    pub fn replay_record(&mut self, ops: &[Operation], payload: &[u8]) -> Result<(), Failure> {
        if !self.complete || self.pending.is_some() {
            return Err(Failure::Unavailable);
        }
        let end = frame_end(self.offset, self.sequence, payload.len() as u64)?;

        self.invalidate();
        self.complete = false;
        if let Err(index) = self.indexes.apply(ops) {
            return Err(Failure::InvalidOperation(index));
        }

        self.offset = end;
        self.sequence += 1;
        self.complete = true;
        Ok(())
    }

    pub fn confirm_replay(&mut self, end: u64, sequence: u64) -> Result<(), Failure> {
        if !self.complete
            || self.pending.is_some()
            || end != self.offset
            || sequence != self.sequence
        {
            return Err(Failure::Mismatch);
        }
        if !replay::confirm(&mut self.recovery, end) {
            return Err(Failure::Truncated);
        }
        Ok(())
    }

    pub fn begin_append(&mut self, ops: Vec<Operation>, payload: &[u8]) -> Result<(), Failure> {
        if !self.ready() {
            return Err(Failure::Unavailable);
        }
        let end = frame_end(self.offset, self.sequence, payload.len() as u64)?;
        if let Err(index) = self.indexes.validate(&ops) {
            return Err(Failure::InvalidOperation(index));
        }
        self.invalidate();
        self.pending = Some(Pending {
            operations: ops,
            end,
            sequence: self.sequence + 1,
        });
        Ok(())
    }

    /// Compose admission, the actual byte append driver and confirmation.
    pub fn append_to<D: crate::wal_device::Writer>(
        &mut self,
        device: &mut D,
        operations: Vec<Operation>,
        payload: &[u8],
    ) -> Result<(), AppendFailure<D::Error>> {
        let offset = self.offset();
        let sequence = self.sequence();
        if let Err(error) = self.begin_append(operations, payload) {
            return Err(AppendFailure::State(error));
        }
        if let Err(error) = crate::wal_append::append(
            device,
            crate::framing::Kind::Catalog,
            sequence,
            offset,
            payload,
        ) {
            return Err(AppendFailure::Write(error));
        }
        match self.finish_append() {
            Ok(()) => Ok(()),
            Err(error) => Err(AppendFailure::State(error)),
        }
    }

    pub fn finish_append(&mut self) -> Result<(), Failure> {
        let Some(plan) = self.pending.take() else {
            return Err(Failure::Unavailable);
        };
        self.complete = false;
        if let Err(index) = self.indexes.apply(&plan.operations) {
            return Err(Failure::InvalidOperation(index));
        }

        self.offset = plan.end;
        self.sequence = plan.sequence;
        self.complete = true;
        if !replay::confirm(&mut self.recovery, plan.end) {
            return Err(Failure::Truncated);
        }
        Ok(())
    }
}

fn frame_end(offset: u64, sequence: u64, payload: u64) -> Result<u64, Failure> {
    if sequence == u64::MAX {
        return Err(Failure::Boundary);
    }
    match wal::frame_extent(offset, u64::MAX, payload) {
        wal::FrameExtent::Complete { end } => Ok(end),
        _ => Err(Failure::Boundary),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn owner(name: &str) -> Operation {
        Operation::Owner(name.into(), BTreeSet::from(["raw/pack".into()]))
    }

    #[test]
    fn partial_apply_cannot_be_confirmed_or_continued() {
        let mut state = LogState::new();
        assert_eq!(state.begin_replay(200), Ok(false));
        assert_eq!(
            state.replay_record(
                &[owner("tentative"), Operation::Reclaim("missing".into())],
                &[0; 8]
            ),
            Err(Failure::InvalidOperation(1))
        );
        assert!(state.indexes().owners.contains_key("tentative"));
        assert_eq!(state.offset(), 0);
        assert_eq!(state.sequence(), 0);
        assert_eq!(state.floor(), 0);
        assert!(state.dirty());
        assert_eq!(state.confirm_replay(0, 0), Err(Failure::Mismatch));
        assert_eq!(
            state.replay_record(&[owner("later")], &[0; 8]),
            Err(Failure::Unavailable)
        );
        assert_eq!(
            state.begin_append(vec![owner("later")], &[0; 8]),
            Err(Failure::Unavailable)
        );
        assert_eq!(state.begin_replay(0), Ok(true));
        assert!(state.indexes().owners.is_empty());
        assert_eq!(state.confirm_replay(0, 0), Ok(()));
        assert!(state.ready());
    }

    #[test]
    fn uncertain_append_plan_is_discarded_by_recovery() {
        let mut state = LogState::new();
        assert_eq!(state.begin_append(vec![owner("old-plan")], &[0; 8]), Ok(()));
        assert!(!state.ready());
        assert!(state.indexes().owners.is_empty());
        assert_eq!(
            state.begin_append(vec![owner("later")], &[0; 8]),
            Err(Failure::Unavailable)
        );
        assert_eq!(state.confirm_replay(0, 0), Err(Failure::Mismatch));
        assert_eq!(state.begin_replay(104), Ok(true));
        assert_eq!(state.finish_append(), Err(Failure::Unavailable));
        assert_eq!(state.replay_record(&[owner("replayed")], &[0; 8]), Ok(()));
        assert_eq!(state.confirm_replay(103, 1), Err(Failure::Mismatch));
        assert_eq!(state.confirm_replay(104, 2), Err(Failure::Mismatch));
        assert_eq!(state.floor(), 0);
        assert_eq!(state.confirm_replay(104, 1), Ok(()));
        assert!(state.ready());
        assert!(state.indexes().owners.contains_key("replayed"));
        assert!(!state.indexes().owners.contains_key("old-plan"));
    }

    #[test]
    fn append_applies_the_stored_operations_once() {
        let mut state = LogState::new();
        assert_eq!(
            state.begin_append(
                vec![Operation::Pack("raw/pack".into(), false), owner("reader")],
                &[0; 7]
            ),
            Ok(())
        );
        assert_eq!(state.finish_append(), Ok(()));
        assert_eq!(
            (state.offset(), state.sequence(), state.floor()),
            (103, 1, 103)
        );
        assert!(state.ready());
        assert!(state.indexes().owners.contains_key("reader"));
        assert_eq!(state.indexes().packs.get("raw/pack"), Some(&false));
        assert_eq!(state.finish_append(), Err(Failure::Unavailable));
        assert_eq!(state.begin_replay(102), Err(Failure::Truncated));
        assert_eq!(
            (state.offset(), state.sequence(), state.floor()),
            (103, 1, 103)
        );
        assert_eq!(state.begin_replay(103), Ok(false));
        assert_eq!(state.confirm_replay(103, 1), Ok(()));
    }

    #[test]
    fn failed_admission_and_overflow_leave_the_confirmed_state_usable() {
        let mut state = LogState::new();
        assert_eq!(
            state.begin_append(vec![Operation::Reclaim("missing".into())], &[0; 8]),
            Err(Failure::InvalidOperation(0))
        );
        assert_eq!(frame_end(0, 0, u64::MAX), Err(Failure::Boundary));
        assert_eq!(frame_end(1, 0, u64::MAX - 96), Err(Failure::Boundary));
        assert!(state.ready());
        assert_eq!((state.offset(), state.sequence(), state.floor()), (0, 0, 0));
        assert!(state.indexes().owners.is_empty());
    }
}
