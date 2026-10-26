//! Consumer, batch, checkpoint and retention updates in actual queue replay.
use serde_json::Value;

pub struct Edit {
    pub path: Vec<String>,
    pub replacement: Option<Value>,
}

#[derive(Debug)]
pub enum Error {
    Event,
    State,
}

fn identifier(event: &Value, name: &str) -> Result<String, Error> {
    match crate::catalog_json::get(event, name) {
        Some(Value::String(value)) => Ok(value.clone()),
        _ => Err(Error::Event),
    }
}

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

    if let Some(Value::String(key)) = crate::catalog_json::get(event, "request_key") {
        let digest = crate::request::copy_field(event, "request_digest");
        let response = crate::request::copy_field(event, "response");
        edits.push(Edit {
            path: vec!["requests".to_owned(), key.clone()],
            replacement: Some(Value::Array(vec![digest, response])),
        });
    }

    Ok(edits)
}

pub fn apply(state: &mut Value, event: &Value) -> Result<(), Error> {
    let edits = prepare(event)?;
    let mut remaining = edits.into_iter();

    loop {
        let Some(edit) = remaining.next() else {
            return Ok(());
        };

        if crate::json_update::update(state, &edit.path, edit.replacement).is_err() {
            return Err(Error::State);
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
