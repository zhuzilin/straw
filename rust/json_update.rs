//! Exact nested-object updates used by queue business replay.
use serde_json::Value;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub struct InvalidPath;

#[cfg(straw_verus)]
verus! {
    pub open spec fn fields(value: Value) -> Map<String, Value> {
        match value { Value::Object(map) => crate::json_specs::object(map), _ => Map::empty() }
    }
    pub open spec fn editable(value: Value, path: Seq<String>) -> bool
        decreases path.len(),
    {
        path.len() > 0 && value is Object && (path.len() == 1
            || (fields(value).contains_key(path[0]) && editable(fields(value)[path[0]], path.drop_first())))
    }
    pub open spec fn edited(before: Value, after: Value, path: Seq<String>, replacement: Option<Value>) -> bool
        decreases path.len(),
    {
        path.len() > 0 && before is Object && after is Object
            && if path.len() == 1 {
                fields(after) == match replacement {
                    Some(value) => fields(before).insert(path[0], value),
                    None => fields(before).remove(path[0]),
                }
            } else {
                fields(before).contains_key(path[0]) && fields(after).contains_key(path[0])
                    && fields(after) == fields(before).insert(path[0], fields(after)[path[0]])
                    && edited(fields(before)[path[0]], fields(after)[path[0]], path.drop_first(), replacement)
            }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures result.is_ok() == editable(*old(value), path@),
        result.is_ok() ==> edited(*old(value), *final(value), path@, replacement),
        result.is_err() ==> *final(value) == *old(value),
    decreases path.len(),
))]
pub fn update(
    value: &mut Value,
    path: &[String],
    replacement: Option<Value>,
) -> Result<(), InvalidPath> {
    #[cfg(straw_verus)]
    proof! { broadcast use vstd::std_specs::btree::group_btree_axioms; }
    if path.is_empty() {
        return Err(InvalidPath);
    }
    let Value::Object(map) = value else {
        return Err(InvalidPath);
    };
    if path.len() == 1 {
        match replacement {
            Some(item) => {
                map.insert(path[0].clone(), item);
            }
            None => {
                map.remove(&path[0]);
            }
        }
        return Ok(());
    }
    let Some(child) = map.get_mut(&path[0]) else {
        return Err(InvalidPath);
    };
    update(child, &path[1..], replacement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn invalid_nested_paths_do_not_create_or_modify_objects() {
        for value in [
            Value::Null,
            json!({}),
            json!({"table":null}),
            json!({"table":{"id":7}}),
        ] {
            let mut state = value.clone();
            assert!(
                update(
                    &mut state,
                    &["table".into(), "id".into(), "value".into()],
                    Some(json!(1))
                )
                .is_err()
            );
            assert_eq!(state, value);
        }
        let mut state = json!({"table":{"preserved":[1,2]}});
        let before = state.clone();
        update(&mut state, &["table".into(), "absent".into()], None).unwrap();
        assert_eq!(state, before);
    }
}
