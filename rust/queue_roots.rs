//! Exact queue reference discovery and GC root selection.
//! Digest identity and authenticated transitive closure remain separate boundaries.
use crate::closure_node::Digest;
use serde_json::Value;
use serde_json::value::Index;
use std::collections::HashMap;

pub fn reference(value: &Value) -> bool {
    let Some(manifest) = "manifest".index_into(value) else {
        return false;
    };
    "segment".index_into(manifest).is_some() && "digest".index_into(value).is_some()
}

pub fn references<D: Digest>(
    digester: &mut D,
    value: &Value,
    output: &mut HashMap<String, Value>,
) -> Result<(), D::Error> {
    if reference(value) {
        let key = digester.digest(value)?;
        output.insert(key, value.clone());
    } else {
        if let Some(object) = value.as_object() {
            let mut it = object.iter();

            loop {
                let Some((_, child)) = it.next() else {
                    return Ok(());
                };

                let outcome = references(digester, child, output);

                outcome?;
            }
        } else if let Some(items) = value.as_array() {
            let mut i = 0usize;

            while i < items.len() {
                let outcome = references(digester, &items[i], output);

                outcome?;
                i += 1;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub enum Selection<'a> {
    Task,
    Commit {
        prefix: u64,
        positions: &'a std::collections::HashSet<u64>,
    },
    Batch(&'a [Value]),
}

pub(crate) fn field_value<'a>(value: &'a Value, name: &str) -> &'a Value {
    name.index_into(value).unwrap_or(&Value::Null)
}

pub(crate) fn number(value: &Value, name: &str) -> u64 {
    field_value(value, name).as_u64().unwrap_or(0)
}

pub(crate) fn contains(items: &[Value], value: &Value) -> bool {
    let mut i = 0usize;

    while i < items.len() {
        if Value::eq(&items[i], value) {
            return true;
        }
        i += 1;
    }
    false
}

fn include(value: &Value, selection: Selection<'_>) -> bool {
    match selection {
        Selection::Task => {
            let status = field_value(value, "state").as_str();
            match status {
                Some(name) => name != "completed" && name != "failed" && name != "cancelled",
                None => true,
            }
        }
        Selection::Commit { prefix, positions } => {
            let position = number(value, "position");
            position >= prefix && !positions.contains(&position)
        }
        Selection::Batch(finished) => !contains(finished, field_value(value, "batch_id")),
    }
}

fn selected_input<'a>(value: &'a Value, selection: Selection<'_>) -> &'a Value {
    match selection {
        Selection::Task => field_value(field_value(value, "spec"), "input_ref"),
        Selection::Commit { .. } => field_value(value, "result_ref"),
        Selection::Batch(_) => value,
    }
}

fn collect_item<D: Digest>(
    digester: &mut D,
    value: &Value,
    selection: Selection<'_>,
    output: &mut HashMap<String, Value>,
) -> Result<(), D::Error> {
    if include(value, selection) {
        references(digester, selected_input(value, selection), output)?;
    }
    Ok(())
}

fn collect_array<D: Digest>(
    digester: &mut D,
    values: &[Value],
    selection: Selection<'_>,
    output: &mut HashMap<String, Value>,
) -> Result<(), D::Error> {
    let mut i = 0usize;

    while i < values.len() {
        let outcome = collect_item(digester, &values[i], selection, output);

        outcome?;
        i += 1;
    }

    Ok(())
}

fn collect_object<D: Digest>(
    digester: &mut D,
    value: &Value,
    selection: Selection<'_>,
    output: &mut HashMap<String, Value>,
) -> Result<(), D::Error> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    let mut it = object.iter();

    loop {
        let Some((_, child)) = it.next() else {
            return Ok(());
        };

        let outcome = collect_item(digester, child, selection, output);

        outcome?;
    }
}

#[derive(Debug)]
pub enum Error<E> {
    Array(&'static str),
    Digest(E),
}

pub struct Roots {
    pub references: HashMap<String, Value>,
    pub prefix: u64,
    pub positions: std::collections::HashSet<u64>,
}

pub(crate) fn array<'a, E>(
    value: &'a Value,
    name: &'static str,
) -> Result<&'a Vec<Value>, Error<E>> {
    match field_value(value, name).as_array() {
        Some(items) => Ok(items),
        None => Err(Error::Array(name)),
    }
}

fn positions(items: &[Value]) -> std::collections::HashSet<u64> {
    let mut result = std::collections::HashSet::new();
    let mut i = 0usize;

    while i < items.len() {
        let value = items[i].as_u64();
        if let Some(position) = value {
            result.insert(position);
        }

        i += 1;
    }

    result
}

fn training_selected<D: Digest>(
    digester: &mut D,
    state: &Value,
    progress: &Value,
    prefix: u64,
    positions: &std::collections::HashSet<u64>,
    output: &mut HashMap<String, Value>,
) -> Result<(), Error<D::Error>> {
    let commits = array(state, "commits")?;
    if let Err(error) = collect_array(
        digester,
        commits,
        Selection::Commit { prefix, positions },
        output,
    ) {
        return Err(Error::Digest(error));
    }

    let finished: &[Value] = array(progress, "finished_batches")?;
    let batches = field_value(state, "batches");
    if let Err(error) = collect_object(digester, batches, Selection::Batch(finished), output) {
        return Err(Error::Digest(error));
    }

    Ok(())
}

pub fn training<D: Digest>(
    digester: &mut D,
    state: &Value,
    consumer: &Value,
    progress: &Value,
    output: &mut HashMap<String, Value>,
) -> Result<(), Error<D::Error>> {
    let positions = positions(array(progress, "processed_positions")?);
    training_selected(
        digester,
        state,
        progress,
        number(consumer, "processed_cursor"),
        &positions,
        output,
    )
}

pub fn collect<D: Digest>(
    digester: &mut D,
    state: &Value,
    progress: &Value,
) -> Result<Roots, Error<D::Error>> {
    let consumer = field_value(field_value(state, "consumers"), "training");
    let prefix = number(consumer, "processed_cursor");
    let positions = positions(array(progress, "processed_positions")?);
    let mut roots = HashMap::new();
    if let Err(error) = collect_object(
        digester,
        field_value(state, "tasks"),
        Selection::Task,
        &mut roots,
    ) {
        return Err(Error::Digest(error));
    }
    training_selected(digester, state, progress, prefix, &positions, &mut roots)?;
    if let Err(error) = references(digester, field_value(state, "readers"), &mut roots) {
        return Err(Error::Digest(error));
    }
    if let Err(error) = references(digester, field_value(state, "consumers"), &mut roots) {
        return Err(Error::Digest(error));
    }
    if let Err(error) = references(digester, field_value(state, "producers"), &mut roots) {
        return Err(Error::Digest(error));
    }
    if let Err(error) = references(digester, field_value(state, "checkpoints"), &mut roots) {
        return Err(Error::Digest(error));
    }

    let result = Roots {
        references: roots,
        prefix,
        positions,
    };

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeSet;

    #[derive(Default)]
    struct Hash {
        calls: usize,
        fail_at: Option<usize>,
    }
    impl Digest for Hash {
        type Error = &'static str;
        fn digest(&mut self, value: &Value) -> Result<String, Self::Error> {
            self.calls += 1;
            if self.fail_at == Some(self.calls) {
                return Err("digest unavailable");
            }
            Ok(crate::digest(value).unwrap())
        }
    }
    fn root(name: &str) -> Value {
        json!({"manifest":{"segment":{"path":name}},"digest":name})
    }
    fn names(roots: &HashMap<String, Value>) -> BTreeSet<String> {
        for (key, value) in roots {
            assert_eq!(*key, crate::digest(value).unwrap());
        }
        roots
            .values()
            .map(|value| value["digest"].as_str().unwrap().to_owned())
            .collect()
    }
    fn expected(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }
    fn empty_state() -> Value {
        json!({"tasks":{},"commits":[],"batches":{},"readers":{},"consumers":{},"producers":{},"checkpoints":{}})
    }
    fn empty_progress() -> Value {
        json!({"processed_positions":[],"finished_batches":[]})
    }

    #[test]
    fn nested_references_preserve_existing_and_stop_below_reference() {
        let hidden = root("hidden-below-root");
        let mut outer = root("outer");
        outer["metadata"] = hidden;
        let value = json!({"a":[root("nested"),{"other":root("nested")}],"b":outer,"scalar":42});
        let previous = root("previous");
        let mut roots = HashMap::from([(crate::digest(&previous).unwrap(), previous)]);
        references(&mut Hash::default(), &value, &mut roots).unwrap();
        assert_eq!(names(&roots), expected(&["previous", "nested", "outer"]));
        assert!(reference(
            &json!({"manifest":{"segment":null},"digest":null})
        ));
        assert!(!reference(
            &json!({"manifest":{"not-segment":null},"digest":null})
        ));
    }

    #[test]
    fn traversal_propagates_error_before_later_roots() {
        let prior = root("prior");
        let mut roots = HashMap::from([(crate::digest(&prior).unwrap(), prior)]);
        let mut hash = Hash {
            calls: 0,
            fail_at: Some(2),
        };
        let result = references(
            &mut hash,
            &json!([root("first"), root("failed"), root("later")]),
            &mut roots,
        );
        assert_eq!(result, Err("digest unavailable"));
        assert_eq!(hash.calls, 2);
        assert_eq!(names(&roots), expected(&["prior", "first"]));
    }

    #[test]
    fn roots_select_every_queue_category_and_sparse_training_progress() {
        let state = json!({
            "tasks":{
                "pending":{"state":"pending","spec":{"input_ref":root("pending")}},
                "leased":{"state":"leased","spec":{"input_ref":root("continuation")}},
                "unknown":{"spec":{"input_ref":root("missing-state")}},
                "done":{"state":"completed","spec":{"input_ref":root("completed")}},
                "failed":{"state":"failed","spec":{"input_ref":root("failed")}},
                "cancelled":{"state":"cancelled","spec":{"input_ref":root("cancelled")}}
            },
            "commits":[
                {"position":1,"result_ref":root("below-prefix")},
                {"position":2,"result_ref":root("at-prefix")},
                {"position":3,"result_ref":root("processed-sparse")},
                {"position":4,"result_ref":root("later-commit")}
            ],
            "batches":{
                "open":{"batch_id":"open","plan_ref":root("batch-plan"),"ready_ref":root("batch-ready")},
                "done":{"batch_id":"done","plan_ref":root("finished-batch")}
            },
            "readers":{"old-attempt":{"input_ref":root("old-reader"),"continuation":root("continuation")}},
            "consumers":{"training":{"processed_cursor":2,"state_ref":root("consumer-state"),"progress_ref":root("consumer-progress")}},
            "producers":{"dataset":{"state_ref":root("producer-only")}},
            "checkpoints":{"step":{"checkpoint_ref":root("checkpoint"),"training_dependencies":[root("checkpoint-dependency")]}}
        });
        let progress = json!({"processed_positions":[3,3,"4",null,-1],"finished_batches":["done"]});
        let roots = collect(&mut Hash::default(), &state, &progress).unwrap();
        assert_eq!(roots.prefix, 2);
        assert_eq!(roots.positions, std::collections::HashSet::from([3]));
        assert_eq!(
            names(&roots.references),
            expected(&[
                "pending",
                "continuation",
                "missing-state",
                "at-prefix",
                "later-commit",
                "batch-plan",
                "batch-ready",
                "old-reader",
                "consumer-state",
                "consumer-progress",
                "producer-only",
                "checkpoint",
                "checkpoint-dependency"
            ])
        );
    }

    #[test]
    fn training_roots_use_argument_consumer_and_structural_batch_identity() {
        let state = json!({
            "consumers":{"training":{"processed_cursor":99}},
            "commits":[{"position":0,"result_ref":root("zero")},{"position":1,"result_ref":root("one")}],
            "batches":{"a":{"batch_id":{"part":1},"plan_ref":root("finished")},
                       "b":{"batch_id":{"part":2},"plan_ref":root("live")}}
        });
        let progress = json!({"processed_positions":[],"finished_batches":[{"part":1}]});
        let mut roots = HashMap::new();
        training(
            &mut Hash::default(),
            &state,
            &json!({"processed_cursor":1}),
            &progress,
            &mut roots,
        )
        .unwrap();
        assert_eq!(names(&roots), expected(&["one", "live"]));
    }

    #[test]
    fn malformed_arrays_keep_native_error_precedence_and_fail_closed() {
        let mut state = empty_state();
        state["tasks"]["task"] = json!({"spec":{"input_ref":root("task")}});
        state["commits"] = Value::Null;
        let mut hash = Hash::default();
        assert!(matches!(
            collect(&mut hash, &state, &Value::Null),
            Err(Error::Array("processed_positions"))
        ));
        assert_eq!(hash.calls, 0);
        assert!(matches!(
            collect(&mut hash, &state, &empty_progress()),
            Err(Error::Array("commits"))
        ));
        assert_eq!(hash.calls, 1);
        state["commits"] = json!([{"result_ref":root("commit")}]);
        let progress = json!({"processed_positions":[]});
        let mut roots = HashMap::new();
        assert!(matches!(
            training(&mut hash, &state, &Value::Null, &progress, &mut roots),
            Err(Error::Array("finished_batches"))
        ));
        assert_eq!(names(&roots), expected(&["commit"]));
        state["tasks"] = Value::Null;
        state["batches"] = json!([root("not-an-object")]);
        let result = collect(&mut hash, &state, &empty_progress()).unwrap();
        assert_eq!(names(&result.references), expected(&["commit"]));
    }
}
