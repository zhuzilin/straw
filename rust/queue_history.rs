//! Actual queue event preparation, business application and ordered history.
use crate::{queue_admin, queue_event, queue_replay, queue_task};
use serde_json::value::Index;
use serde_json::{Map, Value};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error {
    Prepare(queue_event::Error),
    Target,
    Body(queue_replay::Error),
    Receipt(queue_admin::Error),
    Transaction,
}

/// These callbacks receive immutable task/event inputs and an independent cache.
/// Business-state safety requires no correctness premise for cache contents.
#[cfg_attr(straw_verus, verus_verify)]
#[allow(clippy::ptr_arg)]
pub trait Caches: queue_replay::Deadlines {
    fn before(&mut self, id: &String, task: &Value);
    fn after(&mut self, id: &String, task: &Value);
    fn event(&mut self, kind: &str, event: &Value);
}

#[cfg(straw_verus)]
verus! {
    pub type Business = (Value, String, usize, bool);
    pub open spec fn table_names() -> Set<Seq<char>> {
        set!["tasks"@, "requests"@, "submissions"@, "consumers"@, "batches"@, "checkpoints"@, "producers"@, "readers"@]
    }
    pub open spec fn initial_state(state: Value) -> bool {
        state is Object
            && (forall|name: Seq<char>| table_names().contains(name) ==> match crate::json_specs::field(state, name) {
                Some(Value::Object(fields)) => crate::json_specs::entries(fields).len() == 0, _ => false,
            })
            && (forall|name: Seq<char>| set!["commits"@, "retired_positions"@].contains(name) ==> match crate::json_specs::field(state, name) {
                Some(Value::Array(items)) => items@.len() == 0, _ => false,
            })
            && queue_task::unsigned(state, "retired_prefix"@, 0)
            && (forall|name: Seq<char>| !table_names().contains(name) && !set!["commits"@, "retired_positions"@, "retired_prefix"@].contains(name)
                ==> crate::json_specs::field(state, name).is_none())
    }
    pub open spec fn task(state: Value, id: Seq<char>) -> Value {
        crate::request::field_or_null(crate::request::field_or_null(state, "tasks"@), id)
    }
    pub open spec fn permitted(state: Value, kind: Seq<char>, id: Seq<char>) -> bool {
        kind == "Submitted"@ || task(state, id) is Object
    }
    pub open spec fn admitted(state: Value, event: Value) -> bool {
        queue_event::targets(event).is_some() && forall|i: int| 0 <= i < queue_event::targets(event)->Some_0.len()
            ==> permitted(state, queue_event::text(event, "type"@)->Some_0, queue_event::targets(event)->Some_0[i])
    }
    pub open spec fn prepared(state: Value, event: Value, result: Result<queue_event::Prepared, Error>) -> bool {
        match result {
            Ok(value) => admitted(state, event) && queue_event::prepared(event, value),
            Err(Error::Prepare(_)) => queue_event::targets(event).is_none(),
            Err(Error::Target) => queue_event::targets(event).is_some() && !admitted(state, event),
            _ => false,
        }
    }
    pub closed spec fn completed_at(before: Business, middle: Value, after: Business, event: Value, recorded: bool, result: Result<(), Error>) -> bool {
        queue_replay::body_result(before.0, middle, before.1, after.1, before.2, after.2,
            before.3, after.3, event, queue_event::text(event, "type"@)->Some_0, Ok(recorded))
            && match result {
                Ok(()) => queue_replay::final_result(middle, after.0, event, recorded, Ok(())),
                Err(Error::Receipt(error)) => queue_replay::final_result(middle, after.0, event, recorded, Err(error)),
                _ => false,
            }
    }
    pub closed spec fn completed(before: Business, after: Business, event: Value, result: Result<(), Error>) -> bool {
        exists|middle: Value, recorded: bool| completed_at(before, middle, after, event, recorded, result)
    }
    pub open spec fn event_result(before: Business, after: Business, event: Value, result: Result<(), Error>) -> bool {
        if queue_event::targets(event).is_none() {
            after == before && match result { Err(Error::Prepare(_)) => true, _ => false }
        } else if !admitted(before.0, event) { after == before && result == Err(Error::Target) }
        else {
            completed(before, after, event, result)
                || match result {
                    Err(Error::Body(error)) => queue_replay::body_result(before.0, after.0, before.1, after.1, before.2, after.2,
                        before.3, after.3, event, queue_event::text(event, "type"@)->Some_0, Err(error)),
                    _ => false,
                }
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures initial_state(result),
))]
pub fn initial() -> Value {
    let mut fields = Map::new();
    fields.insert("tasks".to_owned(), Value::Object(Map::new()));
    fields.insert("requests".to_owned(), Value::Object(Map::new()));
    fields.insert("submissions".to_owned(), Value::Object(Map::new()));
    fields.insert("consumers".to_owned(), Value::Object(Map::new()));
    fields.insert("batches".to_owned(), Value::Object(Map::new()));
    fields.insert("checkpoints".to_owned(), Value::Object(Map::new()));
    fields.insert("commits".to_owned(), Value::Array(Vec::new()));
    fields.insert("producers".to_owned(), Value::Object(Map::new()));
    fields.insert("retired_prefix".to_owned(), Value::Number(0u64.into()));
    fields.insert("retired_positions".to_owned(), Value::Array(Vec::new()));
    fields.insert("readers".to_owned(), Value::Object(Map::new()));
    Value::Object(fields)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures *result == task(*state, id@),
))]
#[allow(clippy::ptr_arg)]
fn lookup<'a>(state: &'a Value, id: &String) -> &'a Value {
    let tasks = "tasks".index_into(state).unwrap_or(&Value::Null);
    id.as_str().index_into(tasks).unwrap_or(&Value::Null)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures prepared(*state, *event, result),
))]
fn prepare<'a, C: Caches>(
    state: &Value,
    event: &'a Value,
    caches: &mut C,
) -> Result<queue_event::Prepared<'a>, Error> {
    let prepared = match queue_event::prepare(event) {
        Ok(prepared) => prepared,
        Err(error) => return Err(Error::Prepare(error)),
    };
    let mut index = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant index <= prepared.affected.len(), queue_event::prepared(*event, prepared),
            forall|i: int| 0 <= i < index ==> permitted(*state, prepared.kind@, prepared.affected@[i]@),
        decreases prepared.affected.len() - index,
    ))]
    while index < prepared.affected.len() {
        let id = &prepared.affected[index];
        let selected = lookup(state, id);
        if !queue_task::valid_target(prepared.kind, selected) {
            #[cfg(straw_verus)]
            proof! { assert(!permitted(*state, prepared.kind@, queue_event::targets(*event)->Some_0[index as int])); }
            return Err(Error::Target);
        }
        caches.before(id, selected);
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! {
        assert forall|i: int| 0 <= i < queue_event::targets(*event)->Some_0.len() implies
            permitted(*state, queue_event::text(*event, "type"@)->Some_0, queue_event::targets(*event)->Some_0[i]) by {
            assert(prepared.affected@[i]@ == queue_event::targets(*event)->Some_0[i]);
        }
    }
    Ok(prepared)
}

#[cfg_attr(straw_verus, verus_spec(
    ensures true,
))]
fn refresh<C: Caches>(
    state: &Value,
    affected: &[String],
    kind: &str,
    event: &Value,
    caches: &mut C,
) {
    let mut index = 0usize;
    #[cfg_attr(straw_verus, verus_spec(invariant index <= affected.len(), decreases affected.len() - index))]
    while index < affected.len() {
        let id = &affected[index];
        caches.after(id, lookup(state, id));
        index += 1;
    }
    caches.event(kind, event);
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures event_result((*old(state), *old(epoch), *old(order), *old(sealed)),
        (*final(state), *final(epoch), *final(order), *final(sealed)), *event, result),
))]
#[allow(clippy::too_many_arguments)]
pub fn apply<C: Caches>(
    state: &mut Value,
    epoch: &mut String,
    order: &mut usize,
    sealed: &mut bool,
    event: &Value,
    caches: &mut C,
) -> Result<(), Error> {
    let prepared = prepare(state, event, caches)?;
    let body_outcome =
        queue_replay::body(state, epoch, order, sealed, event, prepared.kind, caches);
    let recorded = match body_outcome {
        Ok(recorded) => recorded,
        Err(error) => return Err(Error::Body(error)),
    };
    #[cfg(straw_verus)]
    proof! {
        assert(body_outcome == Ok(recorded));
        assert(queue_replay::body_result(*old(state), *state, *old(epoch), *epoch, *old(order), *order, *old(sealed), *sealed,
            *event, queue_event::text(*event, "type"@)->Some_0, Ok(recorded)));
    }
    #[cfg(straw_verus)]
    let middle = verus_exec_expr!(Ghost(*state));
    refresh(state, &prepared.affected, prepared.kind, event, caches);
    let outcome = queue_replay::finish(state, event, recorded);
    #[cfg(straw_verus)]
    proof! { assert(outcome->Ok_0 == ()); }
    let result = match outcome {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Receipt(error)),
    };
    #[cfg(straw_verus)]
    proof! {
        assert(queue_replay::final_result(middle@, *state, *event, recorded, outcome));
        assert(match result {
            Ok(()) => queue_replay::final_result(middle@, *state, *event, recorded, Ok(())),
            Err(Error::Receipt(error)) => queue_replay::final_result(middle@, *state, *event, recorded, Err(error)),
            _ => false,
        });
        assert(queue_replay::body_result(*old(state), middle@, *old(epoch), *epoch, *old(order), *order, *old(sealed), *sealed,
            *event, queue_event::text(*event, "type"@)->Some_0, Ok(recorded)));
        reveal(completed_at);
        assert(completed_at((*old(state), *old(epoch), *old(order), *old(sealed)), middle@,
            (*state, *epoch, *order, *sealed), *event, recorded, result));
        reveal(completed);
        assert(exists|intermediate: Value, flag: bool| completed_at(
            (*old(state), *old(epoch), *old(order), *old(sealed)), intermediate,
            (*state, *epoch, *order, *sealed), *event, flag, result));
    }
    result
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub struct Failure {
    pub index: usize,
    pub error: Error,
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn trace(before: Business, after: Business, events: Seq<Value>, states: Seq<Business>) -> bool {
        states.len() == events.len() + 1 && states[0] == before && states.last() == after
            && forall|i: int| 0 <= i < events.len() ==> event_result(states[i], states[i + 1], events[i], Ok(()))
    }
    pub closed spec fn folded(before: Business, after: Business, events: Seq<Value>) -> bool {
        exists|states: Seq<Business>| trace(before, after, events, states)
    }
    pub proof fn fold_empty(state: Business)
        ensures folded(state, state, Seq::empty()),
    {
        reveal(folded);
        assert(trace(state, state, Seq::empty(), seq![state]));
    }
    pub proof fn fold_push(before: Business, middle: Business, after: Business, events: Seq<Value>, event: Value)
        requires folded(before, middle, events), event_result(middle, after, event, Ok(())),
        ensures folded(before, after, events.push(event)),
    {
        reveal(folded);
        let states = choose|states: Seq<Business>| trace(before, middle, events, states);
        assert forall|i: int| 0 <= i < events.push(event).len() implies
            event_result(states.push(after)[i], states.push(after)[i + 1], events.push(event)[i], Ok(())) by {
            if i < events.len() { assert(event_result(states[i], states[i + 1], events[i], Ok(()))); }
            else { assert(states[i] == middle); }
        }
        assert(trace(before, after, events.push(event), states.push(after)));
    }
    pub proof fn fold_states(before: Business, after: Business, events: Seq<Value>) -> (states: Seq<Business>)
        requires folded(before, after, events),
        ensures trace(before, after, events, states),
    {
        reveal(folded);
        choose|states: Seq<Business>| trace(before, after, events, states)
    }
    pub proof fn fold_join(before: Business, middle: Business, after: Business, left: Seq<Value>, right: Seq<Value>)
        requires folded(before, middle, left), folded(middle, after, right),
        ensures folded(before, after, left + right),
    {
        reveal(folded);
        let a = fold_states(before, middle, left);
        let b = fold_states(middle, after, right);
        let states = a.take(left.len() as int) + b;
        assert forall|i: int| 0 <= i < (left + right).len() implies
            event_result(states[i], states[i + 1], (left + right)[i], Ok(())) by {
            if i < left.len() {
                assert(states[i] == a[i]);
                assert(states[i + 1] == a[i + 1]);
                assert(event_result(a[i], a[i + 1], left[i], Ok(())));
            } else {
                assert(event_result(b[i - left.len()], b[i - left.len() + 1], right[i - left.len()], Ok(())));
            }
        }
        assert(trace(before, after, left + right, states));
    }
    pub closed spec fn failed_at(before: Business, middle: Business, after: Business, events: Seq<Value>, failure: Failure) -> bool {
        failure.index < events.len() && folded(before, middle, events.take(failure.index as int))
            && event_result(middle, after, events[failure.index as int], Err(failure.error))
    }
    pub open spec fn transaction_result(before: Business, after: Business, value: Value, result: Result<(), Failure>) -> bool {
        match value {
            Value::Array(events) => match result {
                Ok(()) => folded(before, after, events@),
                Err(failure) => exists|middle: Business| failed_at(before, middle, after, events@, failure),
            },
            _ => after == before && match result { Err(failure) => failure.index == 0 && failure.error == Error::Transaction, _ => false },
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures transaction_result((*old(state), *old(epoch), *old(order), *old(sealed)),
        (*final(state), *final(epoch), *final(order), *final(sealed)), *value, result),
))]
pub fn transaction<C: Caches>(
    state: &mut Value,
    epoch: &mut String,
    order: &mut usize,
    sealed: &mut bool,
    value: &Value,
    caches: &mut C,
) -> Result<(), Failure> {
    let Value::Array(events) = value else {
        return Err(Failure {
            index: 0,
            error: Error::Transaction,
        });
    };
    let mut index = 0usize;
    #[cfg(straw_verus)]
    proof! { assert(events@.take(0) =~= Seq::<Value>::empty()); fold_empty((*state, *epoch, *order, *sealed)); }
    #[cfg_attr(straw_verus, verus_spec(
        invariant index <= events.len(), crate::environment::string_order_contract(),
            match *value { Value::Array(items) => items@ == events@, _ => false },
            folded((*old(state), *old(epoch), *old(order), *old(sealed)), (*state, *epoch, *order, *sealed), events@.take(index as int)),
        decreases events.len() - index,
    ))]
    while index < events.len() {
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost((*state, *epoch, *order, *sealed)));
        let outcome = apply(state, epoch, order, sealed, &events[index], caches);
        #[cfg(straw_verus)]
        proof! { assert(outcome->Ok_0 == ()); }
        if let Err(error) = outcome {
            let failure = Failure { index, error };
            #[cfg(straw_verus)]
            proof! {
                reveal(failed_at);
                assert(failed_at((*old(state), *old(epoch), *old(order), *old(sealed)), before@,
                    (*state, *epoch, *order, *sealed), events@, failure));
                assert(exists|middle: Business| failed_at((*old(state), *old(epoch), *old(order), *old(sealed)), middle,
                    (*state, *epoch, *order, *sealed), events@, failure));
            }
            return Err(failure);
        }
        #[cfg(straw_verus)]
        proof! {
            fold_push((*old(state), *old(epoch), *old(order), *old(sealed)), before@, (*state, *epoch, *order, *sealed), events@.take(index as int), events@[index as int]);
            assert(events@.take(index as int + 1) == events@.take(index as int).push(events@[index as int]));
        }
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! { assert(events@.take(index as int) == events@); }
    Ok(())
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Recovery {
    type Error;
    fn before(&mut self, index: usize, transaction: &Value) -> Result<(), Self::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum ReplayCause<E> {
    Bookkeeping(E),
    Application(Failure),
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub struct ReplayFailure<E> {
    pub index: usize,
    pub cause: ReplayCause<E>,
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn frame_trace(before: Business, after: Business, frames: Seq<Value>, states: Seq<Business>) -> bool {
        states.len() == frames.len() + 1 && states[0] == before && states.last() == after
            && forall|i: int| 0 <= i < frames.len() ==> transaction_result(states[i], states[i + 1], frames[i], Ok(()))
    }
    pub closed spec fn frames_applied(before: Business, after: Business, frames: Seq<Value>) -> bool {
        exists|states: Seq<Business>| frame_trace(before, after, frames, states)
    }
    pub proof fn frames_empty(state: Business)
        ensures frames_applied(state, state, Seq::empty()),
    {
        reveal(frames_applied);
        assert(frame_trace(state, state, Seq::empty(), seq![state]));
    }
    pub proof fn frames_push(before: Business, middle: Business, after: Business, frames: Seq<Value>, frame: Value)
        requires frames_applied(before, middle, frames), transaction_result(middle, after, frame, Ok(())),
        ensures frames_applied(before, after, frames.push(frame)),
    {
        reveal(frames_applied);
        let states = choose|states: Seq<Business>| frame_trace(before, middle, frames, states);
        assert forall|i: int| 0 <= i < frames.push(frame).len() implies
            transaction_result(states.push(after)[i], states.push(after)[i + 1], frames.push(frame)[i], Ok(())) by {
            if i < frames.len() { assert(transaction_result(states[i], states[i + 1], frames[i], Ok(()))); }
            else { assert(states[i] == middle); }
        }
        assert(frame_trace(before, after, frames.push(frame), states.push(after)));
    }
    pub proof fn frames_states(before: Business, after: Business, frames: Seq<Value>) -> (states: Seq<Business>)
        requires frames_applied(before, after, frames),
        ensures frame_trace(before, after, frames, states),
    {
        reveal(frames_applied);
        choose|states: Seq<Business>| frame_trace(before, after, frames, states)
    }
    pub open spec fn event_list(value: Value) -> Seq<Value> {
        match value { Value::Array(items) => items@, _ => Seq::empty() }
    }
    pub open spec fn flatten(frames: Seq<Value>) -> Seq<Value>
        decreases frames.len(),
    {
        if frames.len() == 0 { Seq::empty() }
        else { flatten(frames.drop_last()) + event_list(frames.last()) }
    }
    pub proof fn frames_fold(before: Business, after: Business, frames: Seq<Value>)
        requires frames_applied(before, after, frames),
        ensures folded(before, after, flatten(frames)),
        decreases frames.len(),
    {
        let states = frames_states(before, after, frames);
        if frames.len() == 0 {
            assert(before == after);
            fold_empty(before);
        } else {
            let prefix = frames.drop_last();
            let middle = states[frames.len() - 1];
            assert forall|i: int| 0 <= i < prefix.len() implies
                transaction_result(states.drop_last()[i], states.drop_last()[i + 1], prefix[i], Ok(())) by {
                assert(transaction_result(states[i], states[i + 1], frames[i], Ok(())));
            }
            assert(frame_trace(before, middle, prefix, states.drop_last()));
            reveal(frames_applied);
            assert(frames_applied(before, middle, prefix));
            frames_fold(before, middle, prefix);
            assert(transaction_result(middle, after, frames.last(), Ok(())));
            fold_join(before, middle, after, flatten(prefix), event_list(frames.last()));
        }
    }
    pub open spec fn start(frames: Seq<Value>) -> int { if frames.len() == 0 { 0 } else { 1 } }
    pub open spec fn body_frames(frames: Seq<Value>) -> Seq<Value> { frames.subrange(start(frames), frames.len() as int) }
    pub closed spec fn recovery_failed<E>(before: Business, middle: Business, after: Business, frames: Seq<Value>, failure: ReplayFailure<E>) -> bool {
        start(frames) <= failure.index < frames.len()
            && frames_applied(before, middle, frames.subrange(start(frames), failure.index as int))
            && match failure.cause {
                ReplayCause::Bookkeeping(_) => after == middle,
                ReplayCause::Application(error) => transaction_result(middle, after, frames[failure.index as int], Err(error)),
            }
    }
    pub open spec fn recovery_result<E>(before: Business, after: Business, frames: Seq<Value>, result: Result<(), ReplayFailure<E>>) -> bool {
        match result {
            Ok(()) => frames_applied(before, after, body_frames(frames)),
            Err(failure) => exists|middle: Business| recovery_failed(before, middle, after, frames, failure),
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures recovery_result((*old(state), *old(epoch), *old(order), *old(sealed)),
        (*final(state), *final(epoch), *final(order), *final(sealed)), frames@, result),
))]
#[allow(clippy::too_many_arguments)]
pub fn recover<C: Caches, R: Recovery>(
    state: &mut Value,
    epoch: &mut String,
    order: &mut usize,
    sealed: &mut bool,
    frames: &[Value],
    caches: &mut C,
    bookkeeping: &mut R,
) -> Result<(), ReplayFailure<R::Error>> {
    let first = if frames.is_empty() { 0 } else { 1 };
    let mut index = first;
    #[cfg(straw_verus)]
    proof! { assert(frames@.subrange(first as int, first as int) =~= Seq::<Value>::empty()); frames_empty((*state, *epoch, *order, *sealed)); }
    #[cfg_attr(straw_verus, verus_spec(
        invariant first == start(frames@), first <= index <= frames.len(), crate::environment::string_order_contract(),
            frames_applied((*old(state), *old(epoch), *old(order), *old(sealed)), (*state, *epoch, *order, *sealed), frames@.subrange(first as int, index as int)),
        decreases frames.len() - index,
    ))]
    while index < frames.len() {
        if let Err(error) = bookkeeping.before(index, &frames[index]) {
            let failure = ReplayFailure {
                index,
                cause: ReplayCause::Bookkeeping(error),
            };
            #[cfg(straw_verus)]
            proof! {
                reveal(recovery_failed);
                assert(recovery_failed((*old(state), *old(epoch), *old(order), *old(sealed)), (*state, *epoch, *order, *sealed),
                    (*state, *epoch, *order, *sealed), frames@, failure));
                assert(exists|middle: Business| recovery_failed((*old(state), *old(epoch), *old(order), *old(sealed)), middle,
                    (*state, *epoch, *order, *sealed), frames@, failure));
            }
            return Err(failure);
        }
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost((*state, *epoch, *order, *sealed)));
        let outcome = transaction(state, epoch, order, sealed, &frames[index], caches);
        #[cfg(straw_verus)]
        proof! { assert(outcome->Ok_0 == ()); }
        if let Err(error) = outcome {
            let failure = ReplayFailure {
                index,
                cause: ReplayCause::Application(error),
            };
            #[cfg(straw_verus)]
            proof! {
                reveal(recovery_failed);
                assert(recovery_failed((*old(state), *old(epoch), *old(order), *old(sealed)), before@,
                    (*state, *epoch, *order, *sealed), frames@, failure));
                assert(exists|middle: Business| recovery_failed((*old(state), *old(epoch), *old(order), *old(sealed)), middle,
                    (*state, *epoch, *order, *sealed), frames@, failure));
            }
            return Err(failure);
        }
        #[cfg(straw_verus)]
        proof! {
            frames_push((*old(state), *old(epoch), *old(order), *old(sealed)), before@, (*state, *epoch, *order, *sealed),
                frames@.subrange(first as int, index as int), frames@[index as int]);
            assert(frames@.subrange(first as int, index as int + 1) == frames@.subrange(first as int, index as int).push(frames@[index as int]));
        }
        index += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct Cache(Vec<Value>);
    impl queue_replay::Deadlines for Cache {
        fn remove(&mut self, id: &String) {
            self.0.push(json!(["remove", id]));
        }
    }
    impl Caches for Cache {
        fn before(&mut self, id: &String, task: &Value) {
            self.0.push(json!(["before", id, task["state"]]));
        }
        fn after(&mut self, id: &String, task: &Value) {
            self.0.push(json!(["after", id, task["state"]]));
        }
        fn event(&mut self, kind: &str, _: &Value) {
            self.0.push(json!(["event", kind]));
        }
    }
    #[derive(Default)]
    struct Bookkeeping {
        seen: Vec<(usize, Value)>,
        fail: Option<usize>,
    }
    impl Recovery for Bookkeeping {
        type Error = &'static str;
        fn before(&mut self, index: usize, transaction: &Value) -> Result<(), Self::Error> {
            self.seen.push((index, transaction.clone()));
            if self.fail == Some(index) {
                Err("bookkeeping")
            } else {
                Ok(())
            }
        }
    }
    fn run(state: &mut Value, events: &Value, cache: &mut Cache) -> Result<(), Failure> {
        transaction(state, &mut String::new(), &mut 0, &mut false, events, cache)
    }
    fn submitted() -> Value {
        json!({"type":"Submitted","tasks":[{"task_id":"a","input_ref":"root","max_attempts":5}],
            "request_key":"submit","request_digest":"digest","response":["a"]})
    }

    #[test]
    fn mixed_frames_use_the_actual_journal_values_in_order_and_skip_only_the_header() {
        let frames = vec![
            json!([{"type":"Run","identity":"this is not a business event"}]),
            json!([submitted(), {"type":"Leased","leases":[{"task_id":"a","attempt_id":"lease","worker_id":"worker","generation":1}]}]),
            json!([{"type":"TaskProgress","task_id":"a","input_ref":"next"},
                {"type":"Epoch","epoch":"new","revoked":["a"]}, {"type":"Sealed"}]),
        ];
        let mut journal = crate::journal_state::State::new(false);
        journal.transactions = frames.clone();
        let (mut state, mut epoch, mut order, mut sealed) = (initial(), String::new(), 0, false);
        let mut cache = Cache::default();
        let mut bookkeeping = Bookkeeping::default();
        journal
            .apply_business(
                &mut state,
                &mut epoch,
                &mut order,
                &mut sealed,
                &mut cache,
                &mut bookkeeping,
            )
            .unwrap();
        assert_eq!(
            bookkeeping.seen,
            vec![(1, frames[1].clone()), (2, frames[2].clone())]
        );
        assert_eq!(epoch, "new");
        assert_eq!(order, 1);
        assert!(sealed);
        assert_eq!(state["tasks"]["a"]["state"], "pending");
        assert_eq!(state["tasks"]["a"]["spec"]["input_ref"], "next");
        assert_eq!(state["tasks"]["a"]["failures"], 1);
        assert_eq!(state["requests"]["submit"], json!(["digest", ["a"]]));
        assert_eq!(
            cache
                .0
                .iter()
                .filter(|v| v[0] == "event")
                .cloned()
                .collect::<Vec<_>>(),
            vec![
                json!(["event", "Submitted"]),
                json!(["event", "Leased"]),
                json!(["event", "TaskProgress"]),
                json!(["event", "Epoch"]),
                json!(["event", "Sealed"])
            ]
        );
        assert_eq!(journal.transactions, frames);
        let flat = Value::Array(
            frames[1..]
                .iter()
                .flat_map(|frame| frame.as_array().unwrap().iter().cloned())
                .collect(),
        );
        let (mut direct, mut direct_epoch, mut direct_order, mut direct_sealed) =
            (initial(), String::new(), 0, false);
        transaction(
            &mut direct,
            &mut direct_epoch,
            &mut direct_order,
            &mut direct_sealed,
            &flat,
            &mut Cache::default(),
        )
        .unwrap();
        assert_eq!(
            (state, epoch, order, sealed),
            (direct, direct_epoch, direct_order, direct_sealed)
        );
    }

    #[test]
    fn admission_checks_the_whole_target_list_before_changing_business_state() {
        let mut state = initial();
        run(&mut state, &json!([submitted()]), &mut Cache::default()).unwrap();
        let before = state.clone();
        let mut cache = Cache::default();
        let failure = run(
            &mut state,
            &json!([{"type":"Expired","task_ids":["a","missing"]}]),
            &mut cache,
        )
        .unwrap_err();
        assert_eq!(failure.index, 0);
        assert!(matches!(failure.error, Error::Target));
        assert_eq!(state, before);
        assert_eq!(cache.0, vec![json!(["before", "a", "pending"])]);
        cache.0.clear();
        let failure = run(
            &mut state,
            &json!([{"type":"Expired","task_ids":["a",false]}]),
            &mut cache,
        )
        .unwrap_err();
        assert!(matches!(failure.error, Error::Prepare(_)));
        assert_eq!(state, before);
        assert!(cache.0.is_empty());
    }

    #[test]
    fn transaction_failure_keeps_the_prior_event_and_partial_failing_body() {
        let mut state = initial();
        let mut cache = Cache::default();
        let failure = run(
            &mut state,
            &json!([submitted(),
            {"type":"Leased","leases":[{"task_id":"a"}]}, {"type":"Cancelled","task_id":"a"}]),
            &mut cache,
        )
        .unwrap_err();
        assert_eq!(failure.index, 1);
        assert!(matches!(failure.error, Error::Body(_)));
        assert_eq!(state["tasks"]["a"]["state"], "leased");
        assert_eq!(state["requests"]["submit"], json!(["digest", ["a"]]));
        assert_eq!(state["readers"], json!({}));
        assert_eq!(
            cache.0,
            vec![
                json!(["before", "a", null]),
                json!(["after", "a", "pending"]),
                json!(["event", "Submitted"]),
                json!(["before", "a", "pending"])
            ]
        );
    }

    #[test]
    fn receipt_failure_occurs_after_body_and_cache_refresh_and_stops_the_transaction() {
        let mut state = initial();
        state["requests"] = Value::Null;
        let mut cache = Cache::default();
        let (mut epoch, mut order, mut sealed) = (String::new(), 0, false);
        let failure = transaction(
            &mut state,
            &mut epoch,
            &mut order,
            &mut sealed,
            &json!([submitted(), {"type":"Sealed"}]),
            &mut cache,
        )
        .unwrap_err();
        assert_eq!(failure.index, 0);
        assert!(matches!(failure.error, Error::Receipt(_)));
        assert_eq!(order, 1);
        assert!(!sealed);
        assert_eq!(state["tasks"]["a"]["state"], "pending");
        assert_eq!(state["requests"], Value::Null);
        assert_eq!(
            cache.0,
            vec![
                json!(["before", "a", null]),
                json!(["after", "a", "pending"]),
                json!(["event", "Submitted"])
            ]
        );
    }

    #[test]
    fn recovery_distinguishes_bookkeeping_failure_from_a_partial_transaction() {
        let frames = vec![
            json!([{"type":"Run"}]),
            json!([submitted()]),
            json!([{"type":"Leased","leases":[{"task_id":"a"}]}]),
            json!([{"type":"Sealed"}]),
        ];
        for fail in [Some(2), None] {
            let (mut state, mut epoch, mut order, mut sealed) =
                (initial(), String::new(), 0, false);
            let mut bookkeeping = Bookkeeping {
                fail,
                ..Default::default()
            };
            let failure = recover(
                &mut state,
                &mut epoch,
                &mut order,
                &mut sealed,
                &frames,
                &mut Cache::default(),
                &mut bookkeeping,
            )
            .unwrap_err();
            assert_eq!(failure.index, 2);
            assert_eq!(bookkeeping.seen.len(), 2);
            assert!(!sealed);
            if fail.is_some() {
                assert!(matches!(
                    failure.cause,
                    ReplayCause::Bookkeeping("bookkeeping")
                ));
                assert_eq!(state["tasks"]["a"]["state"], "pending");
            } else {
                assert!(matches!(
                    failure.cause,
                    ReplayCause::Application(Failure {
                        index: 0,
                        error: Error::Body(_)
                    })
                ));
                assert_eq!(state["tasks"]["a"]["state"], "leased");
            }
        }
        let mut state = initial();
        let before = state.clone();
        let failure = run(&mut state, &Value::Null, &mut Cache::default()).unwrap_err();
        assert!(matches!(
            failure,
            Failure {
                index: 0,
                error: Error::Transaction
            }
        ));
        assert_eq!(state, before);
    }
}
