//! Actual retention-only queue WAL/application path used before queue GC.
use crate::{gc_collection::Controls, journal_state::State};
use serde_json::Value;
use std::collections::HashSet;
#[cfg(straw_verus)]
use vstd::{prelude::*, std_specs::iter::IteratorSpec};

#[cfg(straw_verus)]
verus! {
    pub open spec fn numbers(items:Seq<Value>)->Seq<u64> {
        items.map(|i:int,value:Value| match crate::json_specs::json(value) {
            crate::json_specs::Json::Number(Some(number))=>number,
            _=>0,
        })
    }
    pub open spec fn numeric(items:Seq<Value>)->bool {
        forall|i:int| 0 <= i < items.len() ==> crate::json_specs::json(items[i]) is Number
            && (crate::json_specs::json(items[i])->Number_0).is_some()
    }
    pub open spec fn event(value:Value,prefix:u64,positions:Set<u64>)->bool {
        crate::queue_metadata::text(value,"type"@) == Some("RetentionAdvanced"@)
            && crate::json_specs::field(value,"prefix"@).is_some()
            && crate::json_specs::json(crate::json_specs::field(value,"prefix"@)->Some_0) == crate::json_specs::Json::Number(Some(prefix))
            && crate::json_specs::field(value,"positions"@).is_some()
            && crate::json_specs::json(crate::json_specs::field(value,"positions"@)->Some_0) is Array
            && numeric(crate::json_specs::json(crate::json_specs::field(value,"positions"@)->Some_0)->Array_0)
            && numbers(crate::json_specs::json(crate::json_specs::field(value,"positions"@)->Some_0)->Array_0).to_set() == positions
            && crate::request::event_key(value).is_none()
    }
    pub open spec fn transaction(value:Value,prefix:u64,positions:Set<u64>)->bool {
        crate::json_specs::json(value) is Array && crate::json_specs::json(value)->Array_0.len() == 1 && event(crate::json_specs::json(value)->Array_0[0],prefix,positions)
    }

}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures numeric(result@), numbers(result@).to_set() == positions@,
))]
fn position_values(positions: &HashSet<u64>) -> Vec<Value> {
    let mut values = Vec::new();
    let mut entries = positions.iter();
    #[cfg_attr(straw_verus, verus_spec(
        invariant entries.decrease().is_some(), numeric(values@),
            numbers(values@).to_set().union(entries.remaining().unref().to_set()) == positions@,
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        #[cfg(straw_verus)]
        proof! { if entries.remaining().len()>0 { crate::catalog_state::pop_set(entries.remaining()); } }
        let Some(number) = entries.next() else {
            #[cfg(straw_verus)]
            proof! { assert(entries.remaining().unref().to_set() =~= Set::<u64>::empty()); }
            return values;
        };
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(values@));
        values.push(Value::Number((*number).into()));
        #[cfg(straw_verus)]
        proof! {
            assert(numbers(values@) =~= numbers(before@).push(*number));
            numbers(before@).lemma_push_to_set_commute(*number);
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures transaction(result,prefix,positions@),
))]
#[allow(
    clippy::vec_init_then_push,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn events(prefix: u64, positions: &HashSet<u64>) -> Value {
    let values = position_values(positions);
    let mut object = serde_json::Map::new();
    #[cfg(straw_verus)]
    proof! { assert forall|key:Seq<char>| crate::json_specs::field_in(crate::json_specs::entries(object),key).is_none() by {}; }
    object.insert(
        "type".to_owned(),
        Value::String("RetentionAdvanced".to_owned()),
    );
    object.insert("prefix".to_owned(), Value::Number(prefix.into()));
    object.insert("positions".to_owned(), Value::Array(values));
    let value = Value::Object(object);
    #[cfg(straw_verus)]
    proof! {
        assert(crate::queue_metadata::text(value,"type"@) == Some("RetentionAdvanced"@));
        assert(crate::json_specs::field(value,"request_key"@).is_none());
        assert(crate::request::event_key(value).is_none());
        assert(event(value,prefix,positions@));
    }
    let mut result = Vec::new();
    result.push(value);
    Value::Array(result)
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum Error<E> {
    Append(crate::journal_append::Error<E>),
    Before(E),
    Apply(crate::queue_metadata::Error),
    After(E),
    Consume(E),
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Controller<'a, D: Controls> {
    journal: &'a mut State,
    business: &'a mut Value,
    device: &'a mut D,
}

#[cfg(straw_verus)]
verus! {
    impl<D:Controls> Controller<'_,D> {
        pub closed spec fn resource_valid(&self)->bool {
            self.device.valid() && !self.device@.buffered
                && crate::environment::string_order_contract() && crate::environment::string_content_contract()
        }
        #[verifier::prophetic]
        pub closed spec fn returned(&self)->(State,Value,D) {(*final(self.journal),*final(self.business),*final(self.device))}
        pub closed spec fn borrowed(&self)->(State,Value,D) {(*self.journal,*self.business,*self.device)}
        pub closed spec fn queue(&self)->Value {*self.business}
        pub closed spec fn log(&self)->State {*self.journal}
        #[verifier::prophetic]
        pub closed spec fn file_loan(&self)->D::Loan {self.device.loan()}
        pub closed spec fn file(&self)->crate::wal_device::DeviceView {self.device@}
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<'a, D: Controls> Controller<'a, D> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires device.valid(), !device@.buffered,
            crate::environment::string_order_contract(), crate::environment::string_content_contract(),
        ensures result.resource_valid(), result.queue() == *old(business), result.log() == *old(journal), result.file() == old(device)@, result.file_loan() == old(device).loan(),
            *final(journal) == result.returned().0, *final(business) == result.returned().1, *final(device) == result.returned().2,
    ))]
    pub(crate) fn new(journal: &'a mut State, business: &'a mut Value, device: &'a mut D) -> Self {
        Self {
            journal,
            business,
            device,
        }
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).resource_valid(),
        ensures final(self).resource_valid(), final(self).returned() == old(self).returned(), final(self).file_loan() == old(self).file_loan(), final(self).file().identity == old(self).file().identity,
            result.is_ok() ==> !final(self).log().poisoned
                && final(self).file().durable == final(self).log().offset
                && exists|events:Value| transaction(events,prefix,positions@)
                    && crate::journal_append::accepted(old(self).log(),final(self).log(),events,final(self).file().bytes)
                    && crate::queue_metadata::outcome(old(self).queue(),final(self).queue(),crate::json_specs::json(events)->Array_0[0],Ok(())),
            result.is_ok() ==> forall|key:Seq<char>| crate::queue_gc::input_field(key)
                ==> crate::json_specs::field(old(self).queue(),key) == crate::json_specs::field(final(self).queue(),key),
            match result { Err(Error::Before(_)) | Err(Error::Apply(_)) | Err(Error::After(_)) => final(self).log().poisoned, _ => true },
    ))]
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
        #[cfg(straw_verus)]
        proof! { crate::queue_gc::retirement_preserves_inputs(*old(self).business,*self.business,*event); }
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
#[cfg_attr(straw_verus, verus_verify)]
pub struct Retired {
    roots: crate::queue_roots::Roots,
}
#[cfg(straw_verus)]
verus! { impl View for Retired { type V = crate::queue_roots::Roots; closed spec fn view(&self)->Self::V { self.roots } } }
#[cfg_attr(straw_verus, verus_verify)]
impl Retired {
    #[cfg_attr(straw_verus, verus_spec(result => ensures result == self@,))]
    pub(crate) fn into_roots(self) -> crate::queue_roots::Roots {
        self.roots
    }
    #[cfg(all(test, not(straw_verus)))]
    pub(crate) fn fixture(roots: crate::queue_roots::Roots) -> Self {
        Self { roots }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<D: Controls> Controller<'_, D> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).resource_valid(), roots.selected_for(old(self).queue()),
        ensures final(self).resource_valid(), final(self).returned() == old(self).returned(), final(self).file_loan() == old(self).file_loan(),
            result.is_ok() ==> result->Ok_0@ == roots && (result->Ok_0@).selected_for(final(self).queue())
                && !final(self).log().poisoned && final(self).file().durable == final(self).log().offset
                && exists|events:Value| transaction(events,roots.prefix,roots.positions@)
                    && crate::journal_append::accepted(old(self).log(),final(self).log(),events,final(self).file().bytes)
                    && crate::queue_metadata::outcome(old(self).queue(),final(self).queue(),crate::json_specs::json(events)->Array_0[0],Ok(())),
            final(self).file().identity == old(self).file().identity,
            match result { Err(Error::Before(_)) | Err(Error::Apply(_)) | Err(Error::After(_)) => final(self).log().poisoned, _ => true },
    ))]
    pub(crate) fn commit_roots(
        &mut self,
        roots: crate::queue_roots::Roots,
    ) -> Result<crate::queue_gc::Retired, Error<D::Error>> {
        self.execute(roots.prefix, &roots.positions)?;
        #[cfg(straw_verus)]
        proof! { crate::queue_gc::selection_survives_retirement(old(self).queue(),self.queue(),roots); }
        Ok(Retired { roots })
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<D: Controls> Controller<'_, D> {
    #[cfg_attr(straw_verus, verus_spec(
        requires self.resource_valid(),
        ensures self.returned() == self.borrowed(),
            self.returned().0 == self.log(), self.returned().1 == self.queue(),
            self.returned().2@ == self.file(), self.returned().2.valid(), self.returned().2.loan() == self.file_loan(),
    ))]
    pub(crate) fn finish(self) {}
}

/// Execute retirement and return every borrowed state through the checked loan.
#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(device).valid(), !old(device)@.buffered, roots.selected_for(*old(business)),
        crate::environment::string_order_contract(), crate::environment::string_content_contract(),
    ensures final(device).loan() == old(device).loan(), final(device).valid(), !final(device)@.buffered,
        final(device)@.identity == old(device)@.identity,
        result.is_ok() ==> result->Ok_0@ == roots && (result->Ok_0@).selected_for(*final(business))
            && !final(journal).poisoned && final(device)@.durable == final(journal).offset
            && exists|events:Value| transaction(events,roots.prefix,roots.positions@)
                && crate::journal_append::accepted(*old(journal),*final(journal),events,final(device)@.bytes)
                && crate::queue_metadata::outcome(*old(business),*final(business),crate::json_specs::json(events)->Array_0[0],Ok(())),
        match result { Err(Error::Before(_)) | Err(Error::Apply(_)) | Err(Error::After(_)) => final(journal).poisoned, _ => true },
))]
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
