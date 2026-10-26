//! Production queue WAL boundaries and error quarantine.
use crate::framing::Kind;
use crate::wal_device::{Device, Writer};
use serde_json::Value;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub struct State {
    pub sequence: u64,
    pub(crate) offset: u64,
    pub transactions: Vec<Value>,
    pub poisoned: bool,
    pub incomplete_tail: u64,
    pub(crate) read_only: bool,
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum AppendFailure<E> {
    Unavailable,
    Write(crate::wal_append::Failure<E>),
}

#[cfg_attr(straw_verus, verus_verify)]
impl State {
    /// Replay the actual decoded transactions, excluding the identity header.
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::environment::string_order_contract(),
        ensures crate::queue_history::recovery_result(
            (*old(state), *old(epoch), *old(order), *old(sealed)),
            (*final(state), *final(epoch), *final(order), *final(sealed)), self.transactions@, result),
    ))]
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

    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures result.sequence == 0, result.offset == 0, result.transactions@.len() == 0,
            !result.poisoned, result.incomplete_tail == 0, result.read_only == read_only,
    ))]
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

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(device).valid(),
        ensures final(device).loan() == old(device).loan(), final(device).valid(), final(self).read_only == old(self).read_only,
            final(device).valid(), final(device)@.identity == old(device)@.identity,
            forall|i: int| 0 <= i < final(self).transactions@.len()
                ==> crate::journal_recovery::events(final(self).transactions@[i]),
            result.is_err() ==> final(self).poisoned
                && final(self).sequence == old(self).sequence && final(self).offset == old(self).offset,
            result.is_ok() ==> !final(self).poisoned
                && final(self).sequence == final(self).transactions@.len()
                && final(self).sequence == result->Ok_0.sequence
                && final(self).offset == result->Ok_0.end
                && final(self).offset == final(device)@.cursor
                && !final(device)@.buffered
                && final(self).incomplete_tail == result->Ok_0.incomplete,
            result.is_ok() && !old(self).read_only ==> final(self).offset == final(device)@.bytes.len()
                && final(self).offset == final(device)@.durable,
            result.is_ok() ==> exists|raw: Seq<Seq<u8>>| crate::journal_recovery::decoded(raw, final(self).transactions@)
                && crate::wal_recovery::replayed(Kind::Queue, old(device)@.bytes, 0, 0, Seq::empty(), raw)
                && final(self).offset == crate::wal_recovery::run_end(0, raw),
    ))]
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

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(writer).valid(), !old(writer)@.buffered,
        ensures final(self).read_only == old(self).read_only,
            final(self).transactions@ == old(self).transactions@,
            final(self).incomplete_tail == old(self).incomplete_tail,
            final(writer).loan() == old(writer).loan(), final(writer).valid(), !final(writer)@.buffered, final(writer)@.identity == old(writer)@.identity,
            result.is_err() ==> final(self).sequence == old(self).sequence
                && final(self).offset == old(self).offset,
            old(self).poisoned || old(self).read_only ==> result == Err(AppendFailure::Unavailable)
                && final(writer)@ == old(writer)@,
            result == Err(AppendFailure::Unavailable) ==> old(self).poisoned || old(self).read_only,
            !(old(self).poisoned || old(self).read_only) && result.is_err() ==> final(self).poisoned,
            result.is_ok() ==> !final(self).poisoned && !final(self).read_only
                && result->Ok_0 == old(self).sequence
                && final(self).sequence as int == old(self).sequence as int + 1
                && final(self).offset as int == old(self).offset as int + payload@.len() + 96
                && final(self).offset == final(writer)@.cursor
                && final(self).offset == final(writer)@.bytes.len()
                && final(self).offset == final(writer)@.durable,
            result.is_ok() ==> final(writer)@.bytes.take(old(writer)@.bytes.len() as int) == old(writer)@.bytes,
            result.is_ok() ==> crate::wal_recovery::complete_frame(Kind::Queue,
                final(writer)@.bytes, old(self).offset as int, old(self).sequence as nat,
                payload@, final(self).offset as int),
    ))]
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
