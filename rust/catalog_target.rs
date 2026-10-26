//! Catalog replay application over the actual joint log state.
//! The concrete frame decoder binds JSON and run identity to typed history.
use crate::catalog_log::{Failure, LogState};
use crate::catalog_state::Operation;
use crate::wal_device::Target;
#[cfg(straw_verus)]
use crate::wal_device::TargetModel;
use std::collections::HashMap;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub trait Decoder: crate::catalog_frame::DecoderModel {
    type Error;
    /// Drop derived namespace/closure caches when replay restarts at zero.
    /// Namespace resolution semantics remain stable when derived caches clear.
    #[cfg_attr(straw_verus, verus_spec(requires old(self).valid(), ensures final(self).valid(), final(self)@ == old(self)@, final(self).loan() == old(self).loan()))]
    fn reset(&mut self);
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
        ensures final(self).valid(), final(self)@ == old(self)@, final(self).loan() == old(self).loan(),
            result.is_ok() ==> crate::catalog_frame::meaning(old(self)@, raw@,
                crate::catalog_state::edge_view(known@), result->Ok_0@),
    ))]
    fn decode(
        &mut self,
        raw: &[u8],
        known: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<Operation>, Self::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum Error<E> {
    Decode(E),
    State(Failure),
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Apply<'a, D> {
    log: &'a mut LogState,
    decoder: &'a mut D,
}

#[cfg(straw_verus)]
verus! {
    pub closed spec fn interpreted(context: crate::catalog_frame::Context, history: Seq<crate::catalog_log::Record>) -> bool
        decreases history.len(),
    {
        history.len() == 0 || (interpreted(context, history.drop_last())
            && crate::catalog_frame::meaning(context, history.last().payload,
                crate::catalog_log::history_state(history.drop_last()).edges, history.last().operations))
    }
    pub proof fn empty_interpretation(context: crate::catalog_frame::Context)
        ensures interpreted(context, Seq::empty()),
    { reveal(interpreted); }
    pub proof fn interpreted_prefix(context: crate::catalog_frame::Context, history: Seq<crate::catalog_log::Record>)
        requires interpreted(context, history), history.len() > 0,
        ensures interpreted(context, history.drop_last()),
    { reveal(interpreted); }
    pub proof fn interpret_push(context: crate::catalog_frame::Context, history: Seq<crate::catalog_log::Record>, record: crate::catalog_log::Record)
        requires interpreted(context, history), crate::catalog_frame::meaning(context, record.payload,
            crate::catalog_log::history_state(history).edges, record.operations),
        ensures interpreted(context, history.push(record)),
    { reveal(interpreted); assert(history.push(record).drop_last() == history); }
    pub proof fn interpreted_at(context: crate::catalog_frame::Context, history: Seq<crate::catalog_log::Record>, at: nat)
        requires interpreted(context, history), at < history.len(),
        ensures crate::catalog_frame::meaning(context, history[at as int].payload,
            crate::catalog_log::history_state(history.take(at as int)).edges, history[at as int].operations),
        decreases history.len(),
    {
        reveal(interpreted);
        if at + 1 == history.len() {
            assert(history.take(at as int) == history.drop_last());
        } else {
            interpreted_at(context, history.drop_last(), at);
            assert(history.drop_last().take(at as int) == history.take(at as int));
        }
    }
    impl<D> View for Apply<'_, D> {
        type V = Seq<Seq<u8>>;
        closed spec fn view(&self) -> Self::V { crate::catalog_log::payloads(self.log@.history) }
    }
    impl<D: Decoder> crate::wal_device::TargetModel for Apply<'_, D> {
        type Context = (int, crate::catalog_frame::Context);
        type Loan = (crate::catalog_log::State, bool, crate::catalog_frame::Context, D::Loan, D::Loan);
        #[verifier::prophetic]
        closed spec fn loan(&self) -> Self::Loan { (final(self.log)@, final(self.decoder).valid(), final(self.decoder)@, final(self.decoder).loan(), self.decoder.loan()) }
        closed spec fn valid(&self) -> bool {
            self.decoder.valid() && crate::catalog_log::valid(self.log@) && crate::environment::string_hash_contract()
                && crate::environment::string_order_contract()
                && interpreted(self.decoder@, self.log@.history)
        }
        closed spec fn complete(&self) -> bool { self.log@.complete && self.log@.pending.is_none() }
        closed spec fn context(&self) -> Self::Context { (self.log@.floor as int, self.decoder@) }
    }
    impl<D> Apply<'_, D> {
        pub closed spec fn state(&self) -> crate::catalog_log::State { self.log@ }
    }
    impl<D: Decoder> Apply<'_, D> {
        #[verifier::prophetic]
        pub closed spec fn decoder_loan(&self) -> D::Loan { self.decoder.loan() }
        pub closed spec fn interpretation(&self) -> crate::catalog_frame::Context { self.decoder@ }
        pub proof fn business_fold(&self)
            requires self.valid(), self.complete(),
            ensures self.state().indexes == crate::catalog_log::history_state(self.state().history),
                forall|i: nat| i < self.state().history.len() ==> crate::catalog_frame::meaning(
                    self.interpretation(), self.state().history[i as int].payload,
                    crate::catalog_log::history_state(self.state().history.take(i as int)).edges,
                    self.state().history[i as int].operations),
        {
            crate::catalog_log::complete_history(self.log@);
            assert forall|i: nat| i < self.state().history.len() implies crate::catalog_frame::meaning(
                self.interpretation(), self.state().history[i as int].payload,
                crate::catalog_log::history_state(self.state().history.take(i as int)).edges,
                self.state().history[i as int].operations) by {
                interpreted_at(self.decoder@, self.log@.history, i);
            }
        }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<'a, D: Decoder> Apply<'a, D> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(decoder).valid(), crate::catalog_log::valid(old(log)@), crate::environment::string_hash_contract(),
            crate::environment::string_order_contract(),
            interpreted(old(decoder)@, old(log)@.history),
        ensures result.valid(), result.state() == old(log)@, result.interpretation() == old(decoder)@,
            result@ == crate::catalog_log::payloads(old(log)@.history),
            final(log)@ == result.loan().0,
            final(decoder).valid() == result.loan().1,
            final(decoder)@ == result.loan().2,
            final(decoder).loan() == result.loan().3,
            result.decoder_loan() == old(decoder).loan(),
    ))]
    pub fn new(log: &'a mut LogState, decoder: &'a mut D) -> Self {
        Self { log, decoder }
    }
    #[cfg_attr(straw_verus, verus_spec(
        requires self.valid(),
        ensures self.loan().0 == self.state(), self.loan().1,
            self.loan().2 == self.interpretation(), self.loan().3 == self.decoder_loan(),
            crate::catalog_log::valid(self.loan().0), interpreted(self.loan().2, self.loan().0.history),
    ))]
    pub fn finish(self) {}
}

#[cfg_attr(straw_verus, verus_verify)]
impl<D: Decoder> Target for Apply<'_, D> {
    type Error = Error<D::Error>;
    fn apply(&mut self, raw: &[u8]) -> Result<(), Self::Error> {
        #[cfg(straw_verus)]
        proof! { crate::catalog_log::complete_history(self.log@); }
        let operations = match self.decoder.decode(raw, &self.log.indexes().staging_edges) {
            Ok(operations) => operations,
            Err(error) => return Err(Error::Decode(error)),
        };
        #[cfg(straw_verus)]
        proof! {
            let record = crate::catalog_log::Record { operations: operations@, payload: raw@ };
            crate::catalog_log::payloads_push(self.log@.history, record);
            interpret_push(self.decoder@, self.log@.history, record);
        }
        match self.log.replay_record(&operations, raw) {
            Ok(()) => Ok(()),
            Err(error) => Err(Error::State(error)),
        }
    }
}

/// Complete the actual catalog recovery before authorizing its next append.
#[cfg_attr(straw_verus, verus_verify)]
impl<C: Decoder> Apply<'_, C> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(device).valid(), old(self).valid(),
        ensures final(device).loan() == old(device).loan(), final(device).valid(), final(self).valid(),
            final(self).loan() == old(self).loan(), final(self).decoder_loan() == old(self).decoder_loan(), final(device)@.identity == old(device)@.identity,
            final(self).interpretation() == old(self).interpretation(),
            result.is_err() ==> final(self).state().floor == old(self).state().floor,
            result.is_ok() ==> crate::catalog_log::ready(final(self).state())
                && final(self).state().offset == final(device)@.cursor
                && final(self).state().offset == final(device)@.bytes.len()
                && final(self).state().offset == final(device)@.durable
                && final(self).state().offset == result->Ok_0.end
                && final(self).state().sequence == result->Ok_0.sequence
                && final(self).state().floor >= old(self).state().floor
                && !final(device)@.buffered,
            result.is_ok() ==> crate::wal_recovery::replayed(crate::framing::Kind::Catalog,
                old(device)@.bytes,
                if old(self).state().dirty { 0 } else { old(self).state().offset as int },
                if old(self).state().dirty { 0 } else { old(self).state().sequence as nat },
                if old(self).state().dirty { Seq::empty() } else { old(self)@ }, final(self)@),
    ))]
    pub fn recover<D: crate::wal_device::Device>(
        &mut self,
        device: &mut D,
    ) -> Result<
        crate::wal_recovery::Boundary,
        crate::wal_recovery::Failure<D::Error, Error<C::Error>>,
    > {
        let size = match device.length() {
            Ok(size) => size,
            Err(error) => return Err(crate::wal_recovery::Failure::Device(error)),
        };
        let rebuilt = self
            .log
            .begin_replay(size)
            .map_err(|error| crate::wal_recovery::Failure::Apply(Error::State(error)))?;
        if rebuilt {
            self.decoder.reset();
        }
        #[cfg(straw_verus)]
        proof! { if rebuilt { reveal(interpreted); } }
        #[cfg(straw_verus)]
        let base = verus_exec_expr!(Ghost(self.state()));
        #[cfg(straw_verus)]
        proof! {
            if rebuilt { assert(self@ =~= Seq::<Seq<u8>>::empty()); }
            else { assert(self@ == old(self)@); }
        }
        let offset = self.log.offset();
        let sequence = self.log.sequence();
        let durable = self.log.floor();
        let boundary = crate::wal_recovery::recover(
            device,
            self,
            crate::framing::Kind::Catalog,
            offset,
            sequence,
            durable,
            true,
        )?;
        #[cfg(straw_verus)]
        proof! {
            crate::catalog_log::payload_boundary(base@.history, self.state().history);
            assert(self.log@.offset == boundary.end);
            assert(self.log@.sequence == boundary.sequence);
            assert(self.log@.complete && self.log@.pending.is_none());
            assert(self.log@.floor <= boundary.end);
        }
        let confirmed = self.log.confirm_replay(boundary.end, boundary.sequence);
        #[cfg(straw_verus)]
        proof! { assert(confirmed.is_ok()); }
        confirmed.map_err(|error| crate::wal_recovery::Failure::Apply(Error::State(error)))?;
        Ok(boundary)
    }
}
