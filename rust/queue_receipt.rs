//! Completion receipts and producer continuation in actual queue replay.
use serde_json::value::Index;
use serde_json::{Map, Value};

#[derive(Debug)]
pub enum Error {
    Identity,
    State,
}

fn submission(receipt: &Value, lease: &Value) -> Value {
    let mut fields = Map::new();
    fields.insert("receipt".to_owned(), receipt.clone());
    fields.insert("lease".to_owned(), lease.clone());
    Value::Object(fields)
}

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

    {
        let Some(value) = "commits".index_into_mut(state) else {
            return Err(Error::State);
        };
        let Value::Array(items) = value else {
            return Err(Error::State);
        };

        items.push(receipt.clone());
    }

    Ok(())
}

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
