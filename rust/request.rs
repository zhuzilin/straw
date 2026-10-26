//! Actual request-receipt lookup and replay updates used by the coordinator.
use serde_json::{Map, Value};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub struct Conflict;

#[cfg(straw_verus)]
verus! {
    pub struct Receipt {
        pub digest: Option<Seq<char>>,
        pub response: Seq<u8>,
    }
    pub enum Cached { Empty, Invalid, Receipt(Receipt) }
    pub open spec fn cached(value: Value) -> Cached {
        match crate::json_specs::json(value) {
            crate::json_specs::Json::Null => Cached::Empty,
            crate::json_specs::Json::Array(items) => if items.len() == 2 {
                Cached::Receipt(Receipt { digest: crate::json_specs::text(items[0]),
                    response: crate::json_specs::content(items[1]) })
            } else { Cached::Invalid },
            _ => Cached::Invalid,
        }
    }
    pub open spec fn ledger(map: Map<String, Value>) -> vstd::map::Map<String, Cached> {
        let values = crate::json_specs::object(map);
        vstd::map::Map::new(values.dom(), |key: String| cached(values[key]))
    }
    pub open spec fn at(map: Map<String, Value>, key: String) -> Cached {
        if ledger(map).contains_key(key) { ledger(map)[key] } else { Cached::Empty }
    }
    pub open spec fn field_or_null(event: Value, name: Seq<char>) -> Value {
        match crate::json_specs::field(event, name) { Some(value) => value, None => Value::Null }
    }
    pub open spec fn event_key(event: Value) -> Option<String> {
        match crate::json_specs::field(event, "request_key"@) {
            Some(Value::String(key)) => Some(key), _ => None,
        }
    }
    pub open spec fn event_receipt(event: Value) -> Receipt {
        Receipt { digest: crate::json_specs::text(field_or_null(event, "request_digest"@)),
            response: crate::json_specs::content(field_or_null(event, "response"@)) }
    }
    pub open spec fn step(before: vstd::map::Map<String, Cached>, event: Value) -> vstd::map::Map<String, Cached> {
        match event_key(event) { None => before, Some(key) => before.insert(key, Cached::Receipt(event_receipt(event))) }
    }
    pub open spec fn outcome(before: Cached, digest: Seq<char>, result: Result<Option<Value>, Conflict>) -> bool {
        match before {
            Cached::Empty => result == Ok(None),
            Cached::Invalid => result == Err(Conflict),
            Cached::Receipt(receipt) => if receipt.digest == Some(digest) {
                match result { Ok(Some(response)) => crate::json_specs::content(response) == receipt.response, _ => false }
            } else { result == Err(Conflict) },
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures outcome(at(*requests, *key), digest@, result),
))]
// The borrowed String uses the same-key library contract without allocating.
#[allow(clippy::ptr_arg)]
pub fn lookup(
    requests: &Map<String, Value>,
    key: &String,
    digest: &str,
) -> Result<Option<Value>, Conflict> {
    #[cfg(straw_verus)]
    proof! { broadcast use vstd::std_specs::btree::group_btree_axioms; }
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
#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures match at(*requests, *key) {
        Cached::Receipt(receipt) => result.is_some()
            && crate::json_specs::content(*result.unwrap()) == receipt.response,
        _ => result.is_none(),
    },
))]
#[allow(clippy::ptr_arg)]
pub fn response<'a>(requests: &'a Map<String, Value>, key: &String) -> Option<&'a Value> {
    #[cfg(straw_verus)]
    proof! { broadcast use vstd::std_specs::btree::group_btree_axioms; }
    let prior = requests.get(key)?;
    let pair = prior.as_array()?;
    if pair.len() != 2 {
        return None;
    }
    Some(&pair[1])
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures crate::json_specs::content(result) == crate::json_specs::content(field_or_null(*event, name@)),
        crate::json_specs::text(result) == crate::json_specs::text(field_or_null(*event, name@)),
        forall|depth: nat| crate::json_specs::observe(result, depth)
            == crate::json_specs::observe(field_or_null(*event, name@), depth),
))]
pub(crate) fn copy_field(event: &Value, name: &str) -> Value {
    match crate::catalog_json::get(event, name) {
        Some(value) => value.clone(),
        None => Value::Null,
    }
}

#[cfg_attr(straw_verus, verus_spec(
    requires crate::environment::string_order_contract(),
    ensures ledger(*final(requests)) == step(ledger(*old(requests)), *event),
))]
pub fn remember(requests: &mut Map<String, Value>, event: &Value) {
    let Some(Value::String(key)) = crate::catalog_json::get(event, "request_key") else {
        return;
    };
    let digest = copy_field(event, "request_digest");
    let response = copy_field(event, "response");
    let receipt = Value::Array(vec![digest, response]);
    requests.insert(key.clone(), receipt);
    #[cfg(straw_verus)]
    proof! {
        assert(ledger(*final(requests)) =~= step(ledger(*old(requests)), *event));
    }
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
