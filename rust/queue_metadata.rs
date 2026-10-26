//! Consumer, batch, checkpoint and retention updates in actual queue replay.
use serde_json::Value;
#[cfg(straw_verus)]
use vstd::{prelude::*, std_specs::iter::IteratorSpec};

#[cfg_attr(straw_verus, verus_verify)]
pub struct Edit {
    pub path: Vec<String>,
    pub replacement: Option<Value>,
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error {
    Event,
    State,
}

#[cfg(straw_verus)]
verus! {
    pub enum Payload { Value(Value), Delete, Receipt(crate::request::Receipt) }
    pub struct Meaning { pub path: Seq<Seq<char>>, pub payload: Payload }
    pub open spec fn text(event: Value, name: Seq<char>) -> Option<Seq<char>> {
        crate::json_specs::text(crate::request::field_or_null(event, name))
    }
    pub open spec fn schema(event: Value) -> bool {
        let kind = text(event, "type"@);
        (kind == Some("ConsumerState"@) && text(event, "consumer_id"@).is_some())
            || (kind == Some("BatchPlanned"@) && text(event, "batch_id"@).is_some())
            || (kind == Some("BatchReady"@) && text(event, "batch_id"@).is_some() && text(event, "consumer_id"@).is_some())
            || ((kind == Some("Checkpoint"@) || kind == Some("CheckpointReleased"@)) && text(event, "checkpoint_id"@).is_some())
            || kind == Some("RetentionAdvanced"@)
    }
    pub open spec fn business(event: Value) -> Seq<Meaning> {
        let kind = text(event, "type"@);
        if kind == Some("ConsumerState"@) {
            seq![Meaning { path: seq!["consumers"@, text(event, "consumer_id"@)->Some_0],
                payload: Payload::Value(crate::request::field_or_null(event, "state"@)) }]
        } else if kind == Some("BatchPlanned"@) {
            seq![Meaning { path: seq!["batches"@, text(event, "batch_id"@)->Some_0],
                payload: Payload::Value(crate::request::field_or_null(event, "batch"@)) }]
        } else if kind == Some("BatchReady"@) {
            seq![Meaning { path: seq!["batches"@, text(event, "batch_id"@)->Some_0, "ready_ref"@],
                    payload: Payload::Value(crate::request::field_or_null(event, "ready_ref"@)) },
                Meaning { path: seq!["batches"@, text(event, "batch_id"@)->Some_0, "ready"@], payload: Payload::Value(Value::Bool(true)) },
                Meaning { path: seq!["consumers"@, text(event, "consumer_id"@)->Some_0],
                    payload: Payload::Value(crate::request::field_or_null(event, "state"@)) }]
        } else if kind == Some("Checkpoint"@) {
            seq![Meaning { path: seq!["checkpoints"@, text(event, "checkpoint_id"@)->Some_0],
                payload: Payload::Value(crate::request::field_or_null(event, "checkpoint"@)) }]
        } else if kind == Some("CheckpointReleased"@) {
            seq![Meaning { path: seq!["checkpoints"@, text(event, "checkpoint_id"@)->Some_0], payload: Payload::Delete }]
        } else {
            seq![Meaning { path: seq!["retired_prefix"@], payload: Payload::Value(crate::request::field_or_null(event, "prefix"@)) },
                Meaning { path: seq!["retired_positions"@], payload: Payload::Value(crate::request::field_or_null(event, "positions"@)) }]
        }
    }
    pub open spec fn intended(event: Value) -> Seq<Meaning> {
        match crate::request::event_key(event) {
            None => business(event),
            Some(key) => business(event).push(Meaning { path: seq!["requests"@, key@],
                payload: Payload::Receipt(crate::request::event_receipt(event)) }),
        }
    }
    pub open spec fn same_value(left: Value, right: Value) -> bool {
        crate::json_specs::content(left) == crate::json_specs::content(right)
            && forall|depth: nat| crate::json_specs::observe(left, depth) == crate::json_specs::observe(right, depth)
    }
    pub open spec fn edit_matches(edit: Edit, meaning: Meaning) -> bool {
        edit.path@.len() == meaning.path.len()
            && (forall|i: int| 0 <= i < meaning.path.len() ==> edit.path@[i]@ == meaning.path[i])
            && match meaning.payload {
                Payload::Delete => edit.replacement.is_none(),
                Payload::Value(value) => edit.replacement.is_some() && same_value(edit.replacement->Some_0, value),
                Payload::Receipt(receipt) => edit.replacement.is_some()
                    && crate::request::cached(edit.replacement->Some_0) == crate::request::Cached::Receipt(receipt),
            }
    }
    pub open spec fn plan_matches(event: Value, edits: Seq<Edit>) -> bool {
        schema(event) && edits.len() == intended(event).len()
            && forall|i: int| 0 <= i < edits.len() ==> edit_matches(edits[i], intended(event)[i])
    }
    pub open spec fn trace(before: Value, after: Value, edits: Seq<Edit>, states: Seq<Value>) -> bool {
        states.len() == edits.len() + 1 && states[0] == before && states.last() == after
            && forall|i: int| 0 <= i < edits.len() ==> crate::json_update::edited(
                states[i], states[i + 1], edits[i].path@, edits[i].replacement)
    }
    pub closed spec fn applied(before: Value, after: Value, edits: Seq<Edit>) -> bool {
        exists|states: Seq<Value>| trace(before, after, edits, states)
    }
    pub proof fn applied_trace(before: Value, after: Value, edits: Seq<Edit>) -> (states: Seq<Value>)
        requires applied(before, after, edits),
        ensures trace(before, after, edits, states),
    {
        reveal(applied);
        choose|states: Seq<Value>| trace(before, after, edits, states)
    }
    pub proof fn applied_empty(state: Value)
        ensures applied(state, state, Seq::empty()),
    {
        reveal(applied);
        assert(trace(state, state, Seq::empty(), seq![state]));
    }
    pub proof fn applied_push(before: Value, middle: Value, after: Value, edits: Seq<Edit>, edit: Edit)
        requires applied(before, middle, edits), crate::json_update::edited(middle, after, edit.path@, edit.replacement),
        ensures applied(before, after, edits.push(edit)),
    {
        reveal(applied);
        let states = choose|states: Seq<Value>| trace(before, middle, edits, states);
        assert(states.last() == middle);
        assert forall|i: int| 0 <= i < edits.push(edit).len() implies crate::json_update::edited(
            states.push(after)[i], states.push(after)[i + 1], edits.push(edit)[i].path@, edits.push(edit)[i].replacement) by {
            if i < edits.len() {
                assert(crate::json_update::edited(states[i], states[i + 1], edits[i].path@, edits[i].replacement));
            } else { assert(states[i] == middle); }
        }
        assert(trace(before, after, edits.push(edit), states.push(after)));
    }
    pub open spec fn outcome(before: Value, after: Value, event: Value, result: Result<(), Error>) -> bool {
        match result {
            Err(Error::Event) => !schema(event) && before == after,
            Ok(()) => exists|edits: Seq<Edit>| plan_matches(event, edits) && applied(before, after, edits),
            Err(Error::State) => exists|edits: Seq<Edit>, count: nat| plan_matches(event, edits) && count < edits.len()
                && applied(before, after, edits.take(count as int))
                && !crate::json_update::editable(after, edits[count as int].path@),
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == text(*event, name@).is_some(),
        result.is_ok() ==> result->Ok_0@ == text(*event, name@)->Some_0,
        result.is_err() ==> result == Err(Error::Event),
))]
fn identifier(event: &Value, name: &str) -> Result<String, Error> {
    match crate::catalog_json::get(event, name) {
        Some(Value::String(value)) => Ok(value.clone()),
        _ => Err(Error::Event),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == schema(*event),
        result.is_ok() ==> plan_matches(*event, result->Ok_0@),
        result.is_err() ==> result == Err(Error::Event),
))]
pub fn prepare(event: &Value) -> Result<Vec<Edit>, Error> {
    let Some(kind) = crate::catalog_json::get(event, "type").and_then(Value::as_str) else {
        return Err(Error::Event);
    };
    let mut edits = if kind == "ConsumerState" {
        vec![Edit {
            path: vec!["consumers".to_owned(), identifier(event, "consumer_id")?],
            replacement: Some(crate::request::copy_field(event, "state")),
        }]
    } else if kind == "BatchPlanned" {
        vec![Edit {
            path: vec!["batches".to_owned(), identifier(event, "batch_id")?],
            replacement: Some(crate::request::copy_field(event, "batch")),
        }]
    } else if kind == "BatchReady" {
        let batch = identifier(event, "batch_id")?;
        let consumer = identifier(event, "consumer_id")?;
        vec![
            Edit {
                path: vec!["batches".to_owned(), batch.clone(), "ready_ref".to_owned()],
                replacement: Some(crate::request::copy_field(event, "ready_ref")),
            },
            Edit {
                path: vec!["batches".to_owned(), batch, "ready".to_owned()],
                replacement: Some(Value::Bool(true)),
            },
            Edit {
                path: vec!["consumers".to_owned(), consumer],
                replacement: Some(crate::request::copy_field(event, "state")),
            },
        ]
    } else if kind == "Checkpoint" {
        vec![Edit {
            path: vec![
                "checkpoints".to_owned(),
                identifier(event, "checkpoint_id")?,
            ],
            replacement: Some(crate::request::copy_field(event, "checkpoint")),
        }]
    } else if kind == "CheckpointReleased" {
        vec![Edit {
            path: vec![
                "checkpoints".to_owned(),
                identifier(event, "checkpoint_id")?,
            ],
            replacement: None,
        }]
    } else if kind == "RetentionAdvanced" {
        vec![
            Edit {
                path: vec!["retired_prefix".to_owned()],
                replacement: Some(crate::request::copy_field(event, "prefix")),
            },
            Edit {
                path: vec!["retired_positions".to_owned()],
                replacement: Some(crate::request::copy_field(event, "positions")),
            },
        ]
    } else {
        return Err(Error::Event);
    };
    #[cfg(straw_verus)]
    proof! { assert forall|i: int| 0 <= i < edits@.len() implies edit_matches(edits@[i], business(*event)[i]) by {} }
    if let Some(Value::String(key)) = crate::catalog_json::get(event, "request_key") {
        let digest = crate::request::copy_field(event, "request_digest");
        let response = crate::request::copy_field(event, "response");
        edits.push(Edit {
            path: vec!["requests".to_owned(), key.clone()],
            replacement: Some(Value::Array(vec![digest, response])),
        });
    }
    #[cfg(straw_verus)]
    proof! { assert forall|i: int| 0 <= i < edits@.len() implies edit_matches(edits@[i], intended(*event)[i]) by {} }
    Ok(edits)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures outcome(*old(state), *final(state), *event, result),
))]
pub fn apply(state: &mut Value, event: &Value) -> Result<(), Error> {
    let edits = prepare(event)?;
    let mut remaining = edits.into_iter();
    #[cfg(straw_verus)]
    let mut count = verus_exec_expr!(Ghost(0nat));
    #[cfg(straw_verus)]
    proof! { assert(edits@.take(0) =~= Seq::<Edit>::empty()); applied_empty(*state); }
    #[cfg_attr(straw_verus, verus_spec(
        invariant remaining.decrease().is_some(), count@ <= edits@.len(),
            remaining.remaining() == edits@.subrange(count@ as int, edits@.len() as int),
            plan_matches(*event, edits@), applied(*old(state), *state, edits@.take(count@ as int)),
            crate::environment::string_order_contract(),
        decreases remaining.decrease().unwrap(),
    ))]
    loop {
        let Some(edit) = remaining.next() else {
            #[cfg(straw_verus)]
            proof! {
                assert(count@ == edits@.len());
                assert(edits@.take(count@ as int) == edits@);
                assert(outcome(*old(state), *state, *event, Ok(())));
            }
            return Ok(());
        };
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(*state));
        if crate::json_update::update(state, &edit.path, edit.replacement).is_err() {
            #[cfg(straw_verus)]
            proof! {
                assert(edit == edits@[count@ as int]);
                assert(exists|plan: Seq<Edit>, at: nat| plan == edits@ && at == count@
                    && plan_matches(*event, plan) && at < plan.len()
                    && applied(*old(state), *state, plan.take(at as int))
                    && !crate::json_update::editable(*state, plan[at as int].path@));
            }
            return Err(Error::State);
        }
        #[cfg(straw_verus)]
        proof! {
            assert(edit == edits@[count@ as int]);
            applied_push(*old(state), before@, *state, edits@.take(count@ as int), edit);
            assert(edits@.take((count@ + 1) as int) == edits@.take(count@ as int).push(edit));
            let next = count@ + 1; *count.borrow_mut() = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn failed_later_edit_preserves_exact_prefix_and_does_not_install_receipt() {
        let mut state = json!({
            "batches":{"batch":{"plan_ref":{"unchanged":[1,2]},"ready":false}},
            "consumers":false,"requests":{"old":["old-digest",null]},"unrelated":[1,{"x":2}]
        });
        let before = state.clone();
        let event = json!({"type":"BatchReady","batch_id":"batch","consumer_id":"training",
            "ready_ref":{"new":[3,4]},"state":{"processed_cursor":1},
            "request_key":"request","request_digest":"digest","response":{"accepted":true}});
        assert!(matches!(apply(&mut state, &event), Err(Error::State)));
        assert_eq!(state["batches"]["batch"]["ready_ref"], event["ready_ref"]);
        assert_eq!(state["batches"]["batch"]["ready"], true);
        assert_eq!(
            state["batches"]["batch"]["plan_ref"],
            before["batches"]["batch"]["plan_ref"]
        );
        for field in ["consumers", "requests", "unrelated"] {
            assert_eq!(state[field], before[field]);
        }
    }

    #[test]
    fn accepted_metadata_keeps_reply_and_unrelated_state_through_roundtrip() {
        let mut state =
            json!({"consumers":{"other":{"preserved":true}},"requests":{},"tasks":{"t":1}});
        let event = json!({"type":"ConsumerState","consumer_id":"training",
            "state":{"nested":[{"value":9.901469416441159e-145_f64}],"fetch_cursor":12,"processed_cursor":7},
            "request_key":"request","request_digest":"digest","response":null});
        apply(&mut state, &event).unwrap();
        assert_eq!(state["consumers"]["training"], event["state"]);
        assert_eq!(state["requests"]["request"], json!(["digest", null]));
        assert_eq!(state["consumers"]["other"], json!({"preserved":true}));
        assert_eq!(state["tasks"], json!({"t":1}));
        let before = state.clone();
        let replayed_event: Value =
            serde_json::from_slice(&serde_json::to_vec(&event).unwrap()).unwrap();
        apply(&mut state, &replayed_event).unwrap();
        assert_eq!(state, before);
    }
}
