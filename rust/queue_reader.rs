//! Reader protection records used by actual queue replay.
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
    pub open spec fn attempt(lease: Value) -> Option<String> {
        match crate::json_specs::field(lease, "attempt_id"@) {
            Some(Value::String(id)) => Some(id), _ => None,
        }
    }
    pub open spec fn roots(entry: Value) -> Option<Seq<Value>> {
        match crate::json_specs::json(crate::request::field_or_null(entry, "roots"@)) {
            crate::json_specs::Json::Array(items) => Some(items), _ => None,
        }
    }
    pub open spec fn equal_present(items: Seq<Value>, input: Value) -> bool {
        exists|i: int| 0 <= i < items.len() && crate::json_specs::equal(items[i], input)
    }
    pub open spec fn observations_equal(left: Value, right: Value) -> bool {
        forall|depth: nat| crate::json_specs::observe(left, depth) == crate::json_specs::observe(right, depth)
    }
    pub open spec fn covers(items: Seq<Value>, input: Value) -> bool {
        exists|i: int| 0 <= i < items.len() && #[trigger] observations_equal(items[i], input)
    }
    pub open spec fn protected(entry: Value, input: Value) -> bool {
        roots(entry).is_some() && covers(roots(entry)->Some_0, input)
    }
    pub open spec fn created(entry: Value, lease: Value, input: Value) -> bool {
        entry is Object && crate::queue_task::copied(entry, "lease"@, lease)
            && roots(entry).is_some() && roots(entry)->Some_0.len() == 1
            && crate::queue_metadata::same_value(roots(entry)->Some_0[0], input)
            && forall|name: Seq<char>| name != "lease"@ && name != "roots"@
                ==> crate::json_specs::field(entry, name).is_none()
    }
    pub open spec fn extended(before: Value, after: Value, input: Value) -> bool {
        roots(before).is_some() && roots(after).is_some() && protected(after, input)
            && crate::queue_task::unchanged(before, after, set!["roots"@])
            && if equal_present(roots(before)->Some_0, input) { after == before }
                else { roots(after)->Some_0.len() == roots(before)->Some_0.len() + 1
                    && roots(after)->Some_0.drop_last() == roots(before)->Some_0
                    && crate::queue_metadata::same_value(roots(after)->Some_0.last(), input) }
    }
    pub open spec fn progressable(entry: Value) -> bool {
        entry is Object && roots(entry).is_some()
    }
    pub open spec fn table(value: Value) -> vstd::map::Map<String, Value> { crate::json_update::fields(value) }
    pub open spec fn new_or_extended(before: Option<Value>, after: Value, lease: Value, input: Value) -> bool {
        match before {
            None | Some(Value::Null) => created(after, lease, input),
            Some(entry) => extended(entry, after, input),
        }
    }
    pub open spec fn selected(value: Value, id: String) -> Option<Value> {
        if table(value).contains_key(id) { Some(table(value)[id]) } else { None }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures match result { Some(id) => attempt(*lease) == Some(*id), None => attempt(*lease).is_none() },
))]
pub fn identity(lease: &Value) -> Option<&String> {
    match "attempt_id".index_into(lease) {
        Some(Value::String(id)) => Some(id),
        _ => None,
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == equal_present(items@, *input),
        result ==> covers(items@, *input),
))]
fn contains(items: &[Value], input: &Value) -> bool {
    let mut index = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant index <= items.len(), forall|i: int| 0 <= i < index ==> !crate::json_specs::equal(items@[i], *input),
        decreases items.len() - index,
    ))]
    while index < items.len() {
        if items[index] == *input {
            #[cfg(straw_verus)]
            proof! {
                assert(exists|i: int| i == index && 0 <= i < items.len() && observations_equal(items@[i], *input));
            }
            return true;
        }
        index += 1;
    }
    false
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures created(result, *lease, *input),
))]
fn create(lease: &Value, input: &Value) -> Value {
    let mut entry = Map::new();
    entry.insert("lease".to_owned(), lease.clone());
    entry.insert("roots".to_owned(), Value::Array(vec![input.clone()]));
    Value::Object(entry)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == progressable(*old(entry)),
        result.is_ok() ==> extended(*old(entry), *final(entry), *input),
        result.is_err() ==> result == Err(Error::State) && *final(entry) == *old(entry),
))]
fn extend(entry: &mut Value, input: &Value) -> Result<(), Error> {
    let Some(value) = "roots".index_into_mut(entry) else {
        return Err(Error::State);
    };
    let Value::Array(roots) = value else {
        return Err(Error::State);
    };
    #[cfg(straw_verus)]
    let before = verus_exec_expr!(Ghost(roots@));
    if !contains(roots, input) {
        roots.push(input.clone());
        #[cfg(straw_verus)]
        proof! {
            assert(roots@.drop_last() =~= before@);
            assert(observations_equal(roots@[before@.len() as int], *input));
            assert(exists|i: int| i == before@.len() && 0 <= i < roots.len() && observations_equal(roots@[i], *input));
        }
    }
    #[cfg(straw_verus)]
    proof! { assert(covers(roots@, *input)); }
    Ok(())
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures *input == Value::Null ==> result.is_ok() && *final(readers) == *old(readers),
        *input != Value::Null ==> (
            result == Err(Error::Identity) <==> attempt(*lease).is_none()),
        *input != Value::Null && attempt(*lease).is_some() ==> (
            result.is_ok() == (*old(readers) is Object)),
        result.is_err() ==> *final(readers) == *old(readers),
        result.is_ok() && *input != Value::Null ==> *final(readers) is Object
            && table(*final(readers)).contains_key(attempt(*lease)->Some_0)
            && table(*final(readers)) == table(*old(readers)).insert(attempt(*lease)->Some_0, table(*final(readers))[attempt(*lease)->Some_0])
            && created(table(*final(readers))[attempt(*lease)->Some_0], *lease, *input),
))]
pub fn register(readers: &mut Value, lease: &Value, input: &Value) -> Result<(), Error> {
    #[cfg(straw_verus)]
    proof! { broadcast use vstd::std_specs::btree::group_btree_axioms; }
    if input.is_null() {
        return Ok(());
    }
    let Some(id) = identity(lease) else {
        return Err(Error::Identity);
    };
    let Value::Object(table) = readers else {
        return Err(Error::State);
    };
    table.insert(id.clone(), create(lease, input));
    Ok(())
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures (*input == Value::Null || attempt(*lease).is_none()) ==> result.is_ok() && *final(readers) == *old(readers),
        *input != Value::Null && attempt(*lease).is_some() ==> (
            result.is_ok() == (*old(readers) is Object && match selected(*old(readers), attempt(*lease)->Some_0) {
                None | Some(Value::Null) => true, Some(entry) => progressable(entry),
            })),
        result.is_err() ==> result == Err(Error::State) && *final(readers) == *old(readers),
        result.is_ok() && *input != Value::Null && attempt(*lease).is_some() ==> *final(readers) is Object
            && table(*final(readers)).contains_key(attempt(*lease)->Some_0)
            && table(*final(readers)) == table(*old(readers)).insert(attempt(*lease)->Some_0, table(*final(readers))[attempt(*lease)->Some_0])
            && new_or_extended(selected(*old(readers), attempt(*lease)->Some_0),
                table(*final(readers))[attempt(*lease)->Some_0], *lease, *input),
))]
pub fn progress(readers: &mut Value, lease: &Value, input: &Value) -> Result<(), Error> {
    #[cfg(straw_verus)]
    proof! { broadcast use vstd::std_specs::btree::group_btree_axioms; }
    if input.is_null() {
        return Ok(());
    }
    let Some(id) = identity(lease) else {
        return Ok(());
    };
    let Value::Object(table) = readers else {
        return Err(Error::State);
    };
    match table.get_mut(id) {
        Some(entry) => {
            if entry.is_null() {
                *entry = create(lease, input);
            } else {
                extend(entry, input)?;
            }
        }
        None => {
            table.insert(id.clone(), create(lease, input));
        }
    }
    Ok(())
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures result.is_ok() == (*old(readers) is Object),
        result.is_ok() ==> *final(readers) is Object && table(*final(readers)) == table(*old(readers)).remove(*id),
        result.is_err() ==> result == Err(Error::State) && *final(readers) == *old(readers),
))]
#[allow(clippy::ptr_arg)]
pub fn remove(readers: &mut Value, id: &String) -> Result<(), Error> {
    #[cfg(straw_verus)]
    proof! { broadcast use vstd::std_specs::btree::group_btree_axioms; }
    let Value::Object(table) = readers else {
        return Err(Error::State);
    };
    table.remove(id);
    Ok(())
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn named_prefix(leases: Seq<Value>, count: nat) -> bool {
        count <= leases.len() && forall|i: int| 0 <= i < count ==> attempt(leases[i]).is_some()
    }
    pub open spec fn dropped(before: vstd::map::Map<String, Value>, leases: Seq<Value>, count: nat) -> vstd::map::Map<String, Value>
        recommends count <= leases.len(),
        decreases count,
    {
        if count == 0 { before }
        else { dropped(before, leases, (count - 1) as nat).remove(attempt(leases[count - 1])->Some_0) }
    }
    pub open spec fn done_result(before: Value, after: Value, leases: Seq<Value>, result: Result<(), Error>) -> bool {
        match result {
            Err(Error::State) => !(before is Object) && after == before,
            Ok(()) => before is Object && after is Object && named_prefix(leases, leases.len())
                && table(after) == dropped(table(before), leases, leases.len()),
            Err(Error::Identity) => before is Object && after is Object && exists|count: nat|
                count < leases.len() && named_prefix(leases, count) && attempt(leases[count as int]).is_none()
                    && table(after) == dropped(table(before), leases, count),
        }
    }
    pub proof fn extension_preserves_all_roots(before: Value, after: Value, input: Value)
        requires extended(before, after, input),
        ensures protected(after, input),
            forall|i: int| 0 <= i < roots(before)->Some_0.len() ==> i < roots(after)->Some_0.len()
                && roots(after)->Some_0[i] == roots(before)->Some_0[i],
    {
        if !equal_present(roots(before)->Some_0, input) {
            assert forall|i: int| 0 <= i < roots(before)->Some_0.len() implies i < roots(after)->Some_0.len()
                && roots(after)->Some_0[i] == roots(before)->Some_0[i] by {
                assert(roots(after)->Some_0.drop_last()[i] == roots(before)->Some_0[i]);
            }
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures done_result(*old(readers), *final(readers), leases@, result),
))]
pub fn done(readers: &mut Value, leases: &[Value]) -> Result<(), Error> {
    if !matches!(readers, Value::Object(_)) {
        return Err(Error::State);
    }
    let mut index = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant index <= leases.len(), *readers is Object, *old(readers) is Object,
            named_prefix(leases@, index as nat), table(*readers) == dropped(table(*old(readers)), leases@, index as nat),
            crate::environment::string_order_contract(),
        decreases leases.len() - index,
    ))]
    while index < leases.len() {
        let Some(id) = identity(&leases[index]) else {
            #[cfg(straw_verus)]
            proof! { assert(exists|count: nat| count == index && count < leases.len() && named_prefix(leases@, count)
            && attempt(leases@[count as int]).is_none() && table(*readers) == dropped(table(*old(readers)), leases@, count)); }
            return Err(Error::Identity);
        };
        remove(readers, id)?;
        index += 1;
    }
    Ok(())
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == crate::json_specs::equal(crate::request::field_or_null(
        crate::request::field_or_null(*entry, "lease"@), "worker_id"@), *worker),
))]
pub fn matches_worker(entry: &Value, worker: &Value) -> bool {
    let lease = "lease".index_into(entry).unwrap_or(&Value::Null);
    let owner = "worker_id".index_into(lease).unwrap_or(&Value::Null);
    *owner == *worker
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn owned_by(entry: Value, worker: Value) -> bool {
        crate::json_specs::equal(crate::request::field_or_null(crate::request::field_or_null(entry, "lease"@), "worker_id"@), worker)
    }
    pub open spec fn retired(before: Value, after: Value, worker: Value) -> bool {
        before is Object && after is Object && table(after) == vstd::map::Map::new(
            table(before).dom().filter(|id: String| !owned_by(table(before)[id], worker)), |id: String| table(before)[id])
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures *final(entry) == *old(entry), result == !owned_by(*old(entry), *worker),
))]
fn keep_worker(entry: &mut Value, worker: &Value) -> bool {
    !matches_worker(entry, worker)
}

// Use one callback body in both builds. The verifier-only expansion adds its
// contract; the native expansion is an ordinary stable Rust closure.
#[cfg(straw_verus)]
macro_rules! reader_filter {
    ($entry:ident, $worker:ident, $body:block) => {
        verus_exec_expr!(|_: &String, $entry: &mut Value| -> (result: bool)
            ensures *final($entry) == *old($entry), result == !owned_by(*old($entry), *$worker),
            $body)
    };
}
#[cfg(not(straw_verus))]
macro_rules! reader_filter {
    ($entry:ident, $worker:ident, $body:block) => {
        |_: &String, $entry: &mut Value| -> bool { $body }
    };
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == (*old(readers) is Object),
        result.is_ok() ==> retired(*old(readers), *final(readers), *worker),
        result.is_err() ==> result == Err(Error::State) && *final(readers) == *old(readers),
))]
pub fn retire(readers: &mut Value, worker: &Value) -> Result<(), Error> {
    let Value::Object(table) = readers else {
        return Err(Error::State);
    };
    let predicate = reader_filter!(entry, worker, { keep_worker(entry, worker) });
    #[cfg(straw_verus)]
    let before = verus_exec_expr!(Ghost(crate::json_specs::object(*table)));
    #[cfg(straw_verus)]
    proof! {
        assert forall|key: &String, value: &mut Value, keep: bool| call_ensures(predicate, (key, value), keep) implies
            *final(value) == *value && keep == !owned_by(*value, *worker) by {}
    }
    table.retain(predicate);
    #[cfg(straw_verus)]
    proof! {
        let after = crate::json_specs::object(*table);
        assert forall|key: String| before@.contains_key(key) implies
            after.contains_key(key) == !owned_by(before@[key], *worker)
                && (after.contains_key(key) ==> after[key] == before@[key]) by {
            let (value, keep) = choose|value: &mut Value, keep: bool|
                *value == before@[key] && call_ensures(predicate, (&key, value), keep)
                    && keep == after.contains_key(key) && (keep ==> after[key] == *final(value));
        }
        assert(after =~= vstd::map::Map::new(before@.dom().filter(|id: String| !owned_by(before@[id], *worker)), |id: String| before@[id]));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn progress_preserves_roots_and_uses_native_value_equality() {
        let lease = json!({"attempt_id":"a", "worker_id":"old"});
        let mut readers = json!({"peer":{"lease":{"worker_id":"peer"},"roots":["peer-root"]}});
        register(&mut readers, &lease, &json!(-0.0)).unwrap();
        let old = readers.clone();
        progress(
            &mut readers,
            &json!({"attempt_id":"a","worker_id":"new"}),
            &json!(0.0),
        )
        .unwrap();
        assert_eq!(readers, old);
        let input = json!({"nested":[9.901469416441159e-145_f64, {"x":7}]});
        progress(&mut readers, &lease, &input).unwrap();
        assert_eq!(readers["a"]["lease"], lease);
        assert_eq!(readers["a"]["roots"][0], json!(-0.0));
        assert_eq!(readers["a"]["roots"][1], input);
        assert_eq!(readers["peer"], old["peer"]);
        let before = readers.clone();
        progress(&mut readers, &lease, &input).unwrap();
        assert_eq!(readers, before);
        assert!(matches!(
            done(&mut readers, &[lease, json!({"attempt_id":false})]),
            Err(Error::Identity)
        ));
        assert_eq!(readers, json!({"peer":old["peer"]}));
    }
}
