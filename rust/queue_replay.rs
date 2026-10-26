//! Business-event dispatch shared by native queue recovery and formal replay.
use crate::{
    queue_admin as admin, queue_metadata as metadata, queue_state, queue_transition as transition,
};
use serde_json::Value;
use serde_json::value::Index;

#[derive(Debug)]
pub enum Error {
    Admin(admin::Error),
    Transition(transition::Error),
    Metadata(metadata::Error),
    Unknown,
}

/// Native cache maintenance is separate from the business-state relation.
/// The callback has no access to the exclusively borrowed queue state.
pub trait Deadlines {
    #[allow(clippy::ptr_arg)]
    fn remove(&mut self, id: &String);
}

fn apply_item<D: Deadlines>(
    state: &mut Value,
    item: &Value,
    event: &Value,
    leased: bool,
    deadlines: &mut D,
) -> Result<(), Error> {
    if leased {
        let outcome = transition::leased(state, item);

        match outcome {
            Ok(()) => Ok(()),
            Err(error) => Err(Error::Transition(error)),
        }
    } else {
        let id = match queue_state::task_id(item) {
            Ok(id) => id,
            Err(error) => return Err(Error::Admin(admin::Error::Task(error))),
        };
        let outcome = transition::failed(state, id, event);

        match outcome {
            Ok(()) => {
                deadlines.remove(id);
                Ok(())
            }
            Err(error) => Err(Error::Transition(error)),
        }
    }
}

#[allow(clippy::question_mark)]
fn many<D: Deadlines>(
    state: &mut Value,
    items: &[Value],
    event: &Value,
    leased: bool,
    deadlines: &mut D,
) -> Result<(), Error> {
    let mut index = 0usize;

    while index < items.len() {
        if let Err(error) = apply_item(state, &items[index], event, leased, deadlines) {
            return Err(error);
        }

        index += 1;
    }

    Ok(())
}

fn batch<D: Deadlines>(
    state: &mut Value,
    event: &Value,
    leased: bool,
    deadlines: &mut D,
) -> Result<(), Error> {
    let name = if leased { "leases" } else { "task_ids" };
    let items = match admin::array(event, name) {
        Ok(items) => items,
        Err(error) => return Err(Error::Admin(error)),
    };
    many(state, items, event, leased, deadlines)
}

#[allow(clippy::too_many_arguments)]
pub fn body<D: Deadlines>(
    state: &mut Value,
    epoch: &mut String,
    order: &mut usize,
    sealed: &mut bool,
    event: &Value,
    kind: &str,
    deadlines: &mut D,
) -> Result<bool, Error> {
    if kind == "StagingHandoff" {
    } else if kind == "Epoch" {
        match admin::apply_epoch(state, epoch, event) {
            Ok(()) => {}
            Err(e) => return Err(Error::Admin(e)),
        }
    } else if kind == "Submitted" {
        let outcome = admin::submitted_event(state, order, event);

        match outcome {
            Ok(()) => {}
            Err(e) => return Err(Error::Admin(e)),
        }
    } else if kind == "Leased" {
        batch(state, event, true, deadlines)?;
    } else if kind == "Expired" || kind == "Failed" {
        batch(state, event, false, deadlines)?;
    } else if kind == "Cancelled" {
        match transition::cancelled(state, event) {
            Ok(id) => deadlines.remove(id),
            Err(e) => return Err(Error::Transition(e)),
        }
    } else if kind == "Yielded" || kind == "TaskProgress" {
        let yielding = kind == "Yielded";
        match admin::progress(state, order, event, yielding) {
            Ok(id) => {
                if yielding {
                    deadlines.remove(id);
                }
            }
            Err(e) => return Err(Error::Admin(e)),
        }
    } else if kind == "Completed" {
        let receipt = "receipt".index_into(event).unwrap_or(&Value::Null);
        let lease = "lease".index_into(event).unwrap_or(&Value::Null);
        match transition::complete(state, receipt, lease) {
            Ok(id) => deadlines.remove(id),
            Err(e) => return Err(Error::Transition(e)),
        }
    } else if kind == "Sealed" {
        *sealed = true;
    } else if kind == "ConsumerState"
        || kind == "BatchPlanned"
        || kind == "BatchReady"
        || kind == "Checkpoint"
        || kind == "CheckpointReleased"
        || kind == "RetentionAdvanced"
    {
        return match metadata::apply(state, event) {
            Ok(()) => Ok(true),
            Err(e) => Err(Error::Metadata(e)),
        };
    } else if kind == "ReaderDone" {
        match admin::reader_done(state, event) {
            Ok(()) => {}
            Err(e) => return Err(Error::Admin(e)),
        }
    } else if kind == "ReaderRetired" {
        match admin::reader_retired(state, event) {
            Ok(()) => {}
            Err(e) => return Err(Error::Admin(e)),
        }
    } else {
        return Err(Error::Unknown);
    }
    Ok(false)
}

pub fn finish(state: &mut Value, event: &Value, recorded: bool) -> Result<(), admin::Error> {
    if recorded {
        Ok(())
    } else {
        admin::remember(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct Removed(Vec<String>);
    impl Deadlines for Removed {
        fn remove(&mut self, id: &String) {
            self.0.push(id.clone());
        }
    }
    fn state() -> Value {
        json!({"tasks":{},"readers":{},"requests":{},"producers":{},"submissions":{},"commits":[],
            "consumers":{},"batches":{},"checkpoints":{"peer":{"state_ref":"checkpoint-root"}}})
    }
    fn apply(value: &mut Value, event: &Value, removed: &mut Removed) -> Result<bool, Error> {
        body(
            value,
            &mut "epoch".to_owned(),
            &mut 7,
            &mut false,
            event,
            event["type"].as_str().unwrap(),
            removed,
        )
    }

    #[test]
    fn failed_batch_preserves_the_successful_prefix_and_the_failing_items_reader_effect() {
        let mut value = state();
        for id in ["a", "b", "peer"] {
            queue_state::submit(
                &mut value,
                &json!({"task_id":id,"max_attempts":5,"input_ref":id}),
                0,
            )
            .unwrap();
            transition::leased(
                &mut value,
                &json!({"task_id":id,"attempt_id":id,"worker_id":id}),
            )
            .unwrap();
        }
        value["tasks"]["b"]["failures"] = json!(u64::MAX);
        let before = value.clone();
        let mut removed = Removed::default();
        assert!(matches!(
            apply(
                &mut value,
                &json!({"type":"Failed","task_ids":["a","b","peer"],"retryable":true}),
                &mut removed
            ),
            Err(Error::Transition(transition::Error::Task(
                queue_state::Error::Task(crate::queue_task::Error::Counter)
            )))
        ));
        assert_eq!(removed.0, vec!["a"]);
        assert_eq!(value["tasks"]["a"]["failures"], 1);
        assert_eq!(value["tasks"]["b"], before["tasks"]["b"]);
        assert_eq!(value["tasks"]["peer"], before["tasks"]["peer"]);
        assert_eq!(value["readers"], json!({"peer":before["readers"]["peer"]}));
        assert_eq!(value["checkpoints"], before["checkpoints"]);
        assert_eq!(value["requests"], before["requests"]);
    }

    #[test]
    fn lease_batch_stops_after_the_partial_failing_item_and_expiry_keeps_readers() {
        let mut value = state();
        for id in ["a", "b", "later"] {
            queue_state::submit(
                &mut value,
                &json!({"task_id":id,"max_attempts":5,"input_ref":id}),
                0,
            )
            .unwrap();
        }
        let mut removed = Removed::default();
        assert!(matches!(
            apply(
                &mut value,
                &json!({"type":"Leased","leases":[
            {"task_id":"a","attempt_id":"first"},{"task_id":"b"},{"task_id":"later","attempt_id":"last"}]}),
                &mut removed
            ),
            Err(Error::Transition(transition::Error::Reader(
                crate::queue_reader::Error::Identity
            )))
        ));
        assert_eq!(value["tasks"]["a"]["state"], "leased");
        assert_eq!(value["tasks"]["b"]["state"], "leased");
        assert_eq!(value["tasks"]["later"]["state"], "pending");
        let roots = value["readers"].clone();
        apply(
            &mut value,
            &json!({"type":"Expired","task_ids":["a","a"]}),
            &mut removed,
        )
        .unwrap();
        assert_eq!(removed.0, vec!["a", "a"]);
        assert_eq!(value["tasks"]["a"]["failures"], 2);
        assert_eq!(value["readers"], roots);
    }

    #[test]
    fn worker_retirement_preserves_peer_records_and_other_queue_roots() {
        let mut value = state();
        value["readers"] = json!({
            "a":{"lease":{"worker_id":"retiring"},"roots":["old","middle","latest"]},
            "b":{"lease":{"worker_id":"retiring"},"roots":["shared"]},
            "peer":{"lease":{"worker_id":"peer"},"roots":["shared","peer-only"]},
            "absent-worker":{"roots":["unknown"]}
        });
        let mut expected = value.clone();
        expected["readers"].as_object_mut().unwrap().remove("a");
        expected["readers"].as_object_mut().unwrap().remove("b");
        let event = json!({"type":"ReaderRetired","worker_id":"retiring"});
        let mut removed = Removed::default();
        assert!(!apply(&mut value, &event, &mut removed).unwrap());
        assert_eq!(value, expected);
        assert!(removed.0.is_empty());
        apply(&mut value, &event, &mut removed).unwrap();
        assert_eq!(value, expected);
        // Retain native Value indexing/equality semantics for malformed history.
        apply(&mut value, &json!({"type":"ReaderRetired"}), &mut removed).unwrap();
        assert!(value["readers"]["absent-worker"].is_null());
        assert_eq!(value["readers"]["peer"], expected["readers"]["peer"]);
        value["readers"] = Value::Bool(false);
        let before = value.clone();
        assert!(matches!(
            apply(&mut value, &event, &mut removed),
            Err(Error::Admin(admin::Error::Reader(
                crate::queue_reader::Error::State
            )))
        ));
        assert_eq!(value, before);
    }

    #[test]
    fn sealed_and_metadata_dispatch_preserve_other_controls_and_record_receipts_once() {
        let mut value = state();
        let before = value.clone();
        let (mut epoch, mut order, mut sealed) = ("epoch".to_owned(), 11, false);
        let mut removed = Removed::default();
        for kind in ["StagingHandoff", "Sealed"] {
            assert!(
                !body(
                    &mut value,
                    &mut epoch,
                    &mut order,
                    &mut sealed,
                    &json!({"type":kind}),
                    kind,
                    &mut removed
                )
                .unwrap()
            );
            assert_eq!(value, before);
        }
        assert!(sealed);
        let event = json!({"type":"ConsumerState","consumer_id":"training","state":{"processed_cursor":4},
            "request_key":"state/request","request_digest":"digest","response":null});
        assert!(
            body(
                &mut value,
                &mut epoch,
                &mut order,
                &mut sealed,
                &event,
                "ConsumerState",
                &mut removed
            )
            .unwrap()
        );
        assert_eq!(value["consumers"]["training"], event["state"]);
        assert_eq!(value["requests"]["state/request"], json!(["digest", null]));
        assert_eq!((epoch.as_str(), order, sealed), ("epoch", 11, true));
        let before = value.clone();
        assert!(matches!(
            apply(&mut value, &json!({"type":"Unknown"}), &mut removed),
            Err(Error::Unknown)
        ));
        assert_eq!(value, before);
    }
}
