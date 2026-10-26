//! Actual retention-only queue WAL/application path used before queue GC.
use crate::{gc_collection::Controls, journal_state::State};
use serde_json::Value;
use std::collections::HashSet;

fn position_values(positions: &HashSet<u64>) -> Vec<Value> {
    let mut values = Vec::new();
    let mut entries = positions.iter();

    loop {
        let Some(number) = entries.next() else {
            return values;
        };

        values.push(Value::Number((*number).into()));
    }
}

pub fn events(prefix: u64, positions: &HashSet<u64>) -> Value {
    let values = position_values(positions);
    let mut object = serde_json::Map::new();

    object.insert(
        "type".to_owned(),
        Value::String("RetentionAdvanced".to_owned()),
    );
    object.insert("prefix".to_owned(), Value::Number(prefix.into()));
    object.insert("positions".to_owned(), Value::Array(values));
    let value = Value::Object(object);

    let result = vec![value];
    Value::Array(result)
}

pub enum Error<E> {
    Append(crate::journal_append::Error<E>),
    Before(E),
    Apply(crate::queue_metadata::Error),
    After(E),
    Consume(E),
}

pub struct Controller<'a, D: Controls> {
    journal: &'a mut State,
    business: &'a mut Value,
    device: &'a mut D,
}

impl<'a, D: Controls> Controller<'a, D> {
    pub(crate) fn new(journal: &'a mut State, business: &'a mut Value, device: &'a mut D) -> Self {
        Self {
            journal,
            business,
            device,
        }
    }

    pub(crate) fn execute(
        &mut self,
        prefix: u64,
        positions: &HashSet<u64>,
    ) -> Result<(), Error<D::Error>> {
        let events = events(prefix, positions);
        if let Err(error) = crate::journal_append::append(self.journal, self.device, &events) {
            return Err(Error::Append(error));
        }
        crate::queue_application::arm(self.journal);
        if let Err(error) = self.device.point("before_commit_apply") {
            return Err(Error::Before(error));
        }
        let event = &events.as_array().unwrap()[0];
        if let Err(error) = crate::queue_metadata::apply(self.business, event) {
            return Err(Error::Apply(error));
        }

        if let Err(error) = self.device.point("after_commit_apply") {
            return Err(Error::After(error));
        }
        self.journal.poisoned = false;
        if let Err(error) = self.device.point("before_staging_consume") {
            return Err(Error::Consume(error));
        }
        Ok(())
    }
}

/// Exact roots whose retention transaction was durably appended and applied.
/// Only the checked controller constructs this token in production.
pub struct Retired {
    roots: crate::queue_roots::Roots,
}

impl Retired {
    pub(crate) fn into_roots(self) -> crate::queue_roots::Roots {
        self.roots
    }
    #[cfg(test)]
    pub(crate) fn fixture(roots: crate::queue_roots::Roots) -> Self {
        Self { roots }
    }
}

impl<D: Controls> Controller<'_, D> {
    pub(crate) fn commit_roots(
        &mut self,
        roots: crate::queue_roots::Roots,
    ) -> Result<crate::queue_gc::Retired, Error<D::Error>> {
        self.execute(roots.prefix, &roots.positions)?;

        Ok(Retired { roots })
    }
}

impl<D: Controls> Controller<'_, D> {
    pub(crate) fn finish(self) {}
}

/// Execute retirement and return every borrowed state through the checked loan.
pub(crate) fn commit<D: Controls>(
    journal: &mut State,
    business: &mut Value,
    device: &mut D,
    roots: crate::queue_roots::Roots,
) -> Result<Retired, Error<D::Error>> {
    let mut controller = Controller::new(journal, business, device);
    let outcome = controller.commit_roots(roots);
    controller.finish();
    outcome
}
