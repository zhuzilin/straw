//! Completion receipts and producer continuation in actual queue replay.
use serde_json::value::Index;
use serde_json::{Map, Value};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error {
    Identity,
    State,
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn id(receipt: Value) -> Option<String> {
        match crate::json_specs::field(receipt, "submission_id"@) {
            Some(Value::String(id)) => Some(id), _ => None,
        }
    }
    pub open spec fn recorded(value: Value, receipt: Value, lease: Value) -> bool {
        value is Object && crate::queue_task::copied(value, "receipt"@, receipt)
            && crate::queue_task::copied(value, "lease"@, lease)
            && forall|name: Seq<char>| name != "receipt"@ && name != "lease"@ ==> crate::json_specs::field(value, name).is_none()
    }
    pub open spec fn submissions(state: Value) -> Value { crate::request::field_or_null(state, "submissions"@) }
    pub open spec fn commits(state: Value) -> Value { crate::request::field_or_null(state, "commits"@) }
    pub open spec fn submission_step(before: Value, after: Value, receipt: Value, lease: Value) -> bool {
        before is Object && after is Object && submissions(before) is Object && submissions(after) is Object
            && id(receipt).is_some()
            && crate::json_update::fields(submissions(after)).contains_key(id(receipt)->Some_0)
            && crate::json_update::fields(submissions(after)) == crate::json_update::fields(submissions(before)).insert(
                id(receipt)->Some_0, crate::json_update::fields(submissions(after))[id(receipt)->Some_0])
            && recorded(crate::json_update::fields(submissions(after))[id(receipt)->Some_0], receipt, lease)
            && crate::queue_task::unchanged(before, after, set!["submissions"@])
    }
    pub open spec fn commit_step(before: Value, after: Value, receipt: Value) -> bool {
        crate::queue_task::unchanged(before, after, set!["commits"@])
            && match (commits(before), commits(after)) {
                (Value::Array(old_items), Value::Array(new_items)) => new_items@.len() == old_items@.len() + 1
                    && new_items@.drop_last() == old_items@
                    && crate::queue_metadata::same_value(new_items@.last(), receipt),
                _ => false,
            }
    }
    pub open spec fn outcome(before: Value, after: Value, receipt: Value, lease: Value, result: Result<(), Error>) -> bool {
        match result {
            Err(Error::Identity) => id(receipt).is_none() && after == before,
            Err(Error::State) => id(receipt).is_some() && if !(submissions(before) is Object) { after == before }
                else { !(commits(before) is Array) && submission_step(before, after, receipt, lease) },
            Ok(()) => exists|middle: Value| submission_step(before, middle, receipt, lease) && commit_step(middle, after, receipt),
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures recorded(result, *receipt, *lease),
))]
fn submission(receipt: &Value, lease: &Value) -> Value {
    let mut fields = Map::new();
    fields.insert("receipt".to_owned(), receipt.clone());
    fields.insert("lease".to_owned(), lease.clone());
    Value::Object(fields)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures outcome(*old(state), *final(state), *receipt, *lease, result),
        result.is_ok() == (id(*receipt).is_some() && submissions(*old(state)) is Object && commits(*old(state)) is Array),
))]
pub fn record(state: &mut Value, receipt: &Value, lease: &Value) -> Result<(), Error> {
    let Some(Value::String(id)) = "submission_id".index_into(receipt) else {
        return Err(Error::Identity);
    };
    {
        let Some(value) = "submissions".index_into_mut(state) else {
            return Err(Error::State);
        };
        let Value::Object(table) = value else {
            return Err(Error::State);
        };
        table.insert(id.clone(), submission(receipt, lease));
    }
    #[cfg(straw_verus)]
    let middle = verus_exec_expr!(Ghost(*state));
    #[cfg(straw_verus)]
    proof! { assert(submission_step(*old(state), middle@, *receipt, *lease)); }
    {
        let Some(value) = "commits".index_into_mut(state) else {
            return Err(Error::State);
        };
        let Value::Array(items) = value else {
            return Err(Error::State);
        };
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(items@));
        items.push(receipt.clone());
        #[cfg(straw_verus)]
        proof! { assert(items@.drop_last() =~= before@); }
    }
    #[cfg(straw_verus)]
    proof! {
        assert(commit_step(middle@, *state, *receipt));
        assert(exists|mid: Value| mid == middle@ && submission_step(*old(state), mid, *receipt, *lease) && commit_step(mid, *state, *receipt));
    }
    Ok(())
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn producer(event: Value) -> Option<String> {
        match crate::json_specs::field(event, "producer_id"@) { Some(Value::String(id)) => Some(id), _ => None }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures producer(*event).is_none() ==> result.is_ok() && *final(producers) == *old(producers),
        producer(*event).is_some() ==> (result.is_ok() == (*old(producers) is Object)),
        result.is_err() ==> result == Err(Error::State) && *final(producers) == *old(producers),
        result.is_ok() && producer(*event).is_some() ==> *final(producers) is Object
            && crate::json_update::fields(*final(producers)).contains_key(producer(*event)->Some_0)
            && crate::json_update::fields(*final(producers)) == crate::json_update::fields(*old(producers)).insert(
                producer(*event)->Some_0, crate::json_update::fields(*final(producers))[producer(*event)->Some_0])
            && crate::queue_metadata::same_value(crate::json_update::fields(*final(producers))[producer(*event)->Some_0],
                crate::request::field_or_null(*event, "producer_state"@)),
))]
pub fn continue_producer(producers: &mut Value, event: &Value) -> Result<(), Error> {
    let Some(Value::String(id)) = "producer_id".index_into(event) else {
        return Ok(());
    };
    let Value::Object(table) = producers else {
        return Err(Error::State);
    };
    let value = "producer_state".index_into(event).unwrap_or(&Value::Null);
    table.insert(id.clone(), value.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn malformed_commit_list_preserves_the_exact_submission_prefix() {
        let mut state = json!({"submissions":{"peer":{"keep":true}},"commits":false,"tasks":{"task":"unchanged"}});
        let receipt = json!({"submission_id":"accepted", "result_ref":{"nested":[1,2]}});
        let lease = json!({"attempt_id":"attempt"});
        assert!(matches!(
            record(&mut state, &receipt, &lease),
            Err(Error::State)
        ));
        assert_eq!(
            state,
            json!({"submissions":{"peer":{"keep":true}, "accepted":{"receipt":receipt,"lease":lease}},"commits":false,"tasks":{"task":"unchanged"}})
        );
        let before = state.clone();
        assert!(matches!(
            record(&mut state, &json!({"submission_id":7}), &lease),
            Err(Error::Identity)
        ));
        assert_eq!(state, before);
    }
}
