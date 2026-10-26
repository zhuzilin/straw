//! Actual request-receipt lookup and replay updates used by the coordinator.
use serde_json::{Map, Value};

pub struct Conflict;

// The borrowed String uses the same-key library contract without allocating.
#[allow(clippy::ptr_arg)]
pub fn lookup(
    requests: &Map<String, Value>,
    key: &String,
    digest: &str,
) -> Result<Option<Value>, Conflict> {
    let Some(prior) = requests.get(key) else {
        return Ok(None);
    };
    if prior.is_null() {
        return Ok(None);
    }
    let Some(pair) = prior.as_array() else {
        return Err(Conflict);
    };
    if pair.len() != 2 {
        return Err(Conflict);
    }
    let Some(previous) = pair[0].as_str() else {
        return Err(Conflict);
    };
    if previous != digest {
        return Err(Conflict);
    }
    Ok(Some(pair[1].clone()))
}

/// Inspect an accepted response whose original request is itself stored there
/// (checkpoint registration). The caller must still check identity and digest.
#[allow(clippy::ptr_arg)]
pub fn response<'a>(requests: &'a Map<String, Value>, key: &String) -> Option<&'a Value> {
    let prior = requests.get(key)?;
    let pair = prior.as_array()?;
    if pair.len() != 2 {
        return None;
    }
    Some(&pair[1])
}

pub(crate) fn copy_field(event: &Value, name: &str) -> Value {
    match crate::catalog_json::get(event, name) {
        Some(value) => value.clone(),
        None => Value::Null,
    }
}

pub fn remember(requests: &mut Map<String, Value>, event: &Value) {
    let Some(Value::String(key)) = crate::catalog_json::get(event, "request_key") else {
        return;
    };
    let digest = copy_field(event, "request_digest");
    let response = copy_field(event, "response");
    let receipt = Value::Array(vec![digest, response]);
    requests.insert(key.clone(), receipt);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_cached_null_is_a_reply_and_cannot_become_a_fresh_request() {
        let mut requests = Map::new();
        let key = "operation/id".to_string();
        assert!(matches!(lookup(&requests, &key, "digest"), Ok(None)));
        remember(
            &mut requests,
            &json!({"request_key":key,"request_digest":"digest","response":null}),
        );
        assert!(matches!(
            lookup(&requests, &key, "digest"),
            Ok(Some(Value::Null))
        ));
        assert!(lookup(&requests, &key, "different").is_err());
        assert_eq!(response(&requests, &key), Some(&Value::Null));
    }

    #[test]
    fn replay_keeps_nested_responses_and_unrelated_requests() {
        let mut requests = Map::new();
        let reply = json!({"list":[1,null,{"unicode":"恢复"}],"flag":false});
        let event = json!({"request_key":"first","request_digest":"a","response":reply});
        remember(&mut requests, &event);
        remember(
            &mut requests,
            &json!({"request_key":"other","request_digest":"b","response":[]}),
        );
        remember(
            &mut requests,
            &json!({"request_key":42,"response":"ignored"}),
        );
        remember(&mut requests, &event);
        assert_eq!(requests.len(), 2);
        assert!(
            matches!(lookup(&requests, &"first".into(), "a"), Ok(Some(value)) if value == reply)
        );
        assert!(
            matches!(lookup(&requests, &"other".into(), "b"), Ok(Some(value)) if value == json!([]))
        );
    }
}
