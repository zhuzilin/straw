//! Catalog replay application over the actual joint log state.
//! The concrete frame decoder binds JSON and run identity to typed history.
use crate::catalog_log::{Failure, LogState};
use crate::catalog_state::Operation;
use crate::wal_device::Target;

use std::collections::HashMap;

pub trait Decoder {
    type Error;
    /// Drop derived namespace/closure caches when replay restarts at zero.
    /// Namespace resolution semantics remain stable when derived caches clear.
    fn reset(&mut self);

    fn decode(
        &mut self,
        raw: &[u8],
        known: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<Operation>, Self::Error>;
}

pub enum Error<E> {
    Decode(E),
    State(Failure),
}

pub struct Apply<'a, D> {
    log: &'a mut LogState,
    decoder: &'a mut D,
}

impl<'a, D: Decoder> Apply<'a, D> {
    pub fn new(log: &'a mut LogState, decoder: &'a mut D) -> Self {
        Self { log, decoder }
    }

    pub fn finish(self) {}
}

impl<D: Decoder> Target for Apply<'_, D> {
    type Error = Error<D::Error>;
    fn apply(&mut self, raw: &[u8]) -> Result<(), Self::Error> {
        let operations = match self.decoder.decode(raw, &self.log.indexes().staging_edges) {
            Ok(operations) => operations,
            Err(error) => return Err(Error::Decode(error)),
        };

        match self.log.replay_record(&operations, raw) {
            Ok(()) => Ok(()),
            Err(error) => Err(Error::State(error)),
        }
    }
}

/// Complete the actual catalog recovery before authorizing its next append.
impl<C: Decoder> Apply<'_, C> {
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

        let confirmed = self.log.confirm_replay(boundary.end, boundary.sequence);

        confirmed.map_err(|error| crate::wal_recovery::Failure::Apply(Error::State(error)))?;
        Ok(boundary)
    }
}
