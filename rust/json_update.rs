//! Exact nested-object updates used by queue business replay.
use serde_json::Value;

#[derive(Debug)]
pub struct InvalidPath;

pub fn update(
    value: &mut Value,
    path: &[String],
    replacement: Option<Value>,
) -> Result<(), InvalidPath> {
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
