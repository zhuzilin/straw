//! Production queue WAL boundaries and error quarantine.
use crate::framing::Kind;
use crate::wal_device::{Device, Writer};
use serde_json::Value;

pub struct State {
    pub sequence: u64,
    pub(crate) offset: u64,
    pub transactions: Vec<Value>,
    pub poisoned: bool,
    pub incomplete_tail: u64,
    pub(crate) read_only: bool,
}

pub enum AppendFailure<E> {
    Unavailable,
    Write(crate::wal_append::Failure<E>),
}

impl State {
    /// Replay the actual decoded transactions, excluding the identity header.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_business<
        C: crate::queue_history::Caches,
        R: crate::queue_history::Recovery,
    >(
        &self,
        state: &mut Value,
        epoch: &mut String,
        order: &mut usize,
        sealed: &mut bool,
        caches: &mut C,
        bookkeeping: &mut R,
    ) -> Result<(), crate::queue_history::ReplayFailure<R::Error>> {
        crate::queue_history::recover(
            state,
            epoch,
            order,
            sealed,
            &self.transactions,
            caches,
            bookkeeping,
        )
    }

    pub(crate) fn new(read_only: bool) -> Self {
        Self {
            sequence: 0,
            offset: 0,
            transactions: Vec::new(),
            poisoned: false,
            incomplete_tail: 0,
            read_only,
        }
    }

    pub(crate) fn recover<D: Device>(
        &mut self,
        device: &mut D,
    ) -> Result<
        crate::wal_recovery::Boundary,
        crate::wal_recovery::Failure<D::Error, crate::journal_recovery::Error>,
    > {
        self.poisoned = true;
        let boundary =
            crate::journal_recovery::recover(device, &mut self.transactions, !self.read_only)?;
        self.sequence = boundary.sequence;
        self.offset = boundary.end;
        self.incomplete_tail = boundary.incomplete;
        self.poisoned = false;
        Ok(boundary)
    }

    pub(crate) fn append_to<W: Writer>(
        &mut self,
        writer: &mut W,
        payload: &[u8],
    ) -> Result<u64, AppendFailure<W::Error>> {
        if self.poisoned || self.read_only {
            return Err(AppendFailure::Unavailable);
        }
        self.poisoned = true;
        let appended = match crate::wal_append::append(
            writer,
            Kind::Queue,
            self.sequence,
            self.offset,
            payload,
        ) {
            Ok(appended) => appended,
            Err(error) => return Err(AppendFailure::Write(error)),
        };
        self.offset = appended.end;
        self.sequence = appended.sequence;
        self.poisoned = false;
        Ok(self.sequence - 1)
    }
}
