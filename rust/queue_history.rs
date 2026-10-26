//! Actual queue event preparation, business application and ordered history.
use crate::{queue_admin, queue_event, queue_replay, queue_task};
use serde_json::value::Index;
use serde_json::{Map, Value};

#[derive(Debug)]
pub enum Error {
    Prepare(queue_event::Error),
    Target,
    Body(queue_replay::Error),
    Receipt(queue_admin::Error),
    Transaction,
}

/// These callbacks receive immutable task/event inputs and an independent cache.
/// Business-state safety requires no correctness premise for cache contents.
#[allow(clippy::ptr_arg)]
pub trait Caches: queue_replay::Deadlines {
    fn before(&mut self, id: &String, task: &Value);
    fn after(&mut self, id: &String, task: &Value);
    fn event(&mut self, kind: &str, event: &Value);
}

pub fn initial() -> Value {
    let mut fields = Map::new();
    fields.insert("tasks".to_owned(), Value::Object(Map::new()));
    fields.insert("requests".to_owned(), Value::Object(Map::new()));
    fields.insert("submissions".to_owned(), Value::Object(Map::new()));
    fields.insert("consumers".to_owned(), Value::Object(Map::new()));
    fields.insert("batches".to_owned(), Value::Object(Map::new()));
    fields.insert("checkpoints".to_owned(), Value::Object(Map::new()));
    fields.insert("commits".to_owned(), Value::Array(Vec::new()));
    fields.insert("producers".to_owned(), Value::Object(Map::new()));
    fields.insert("retired_prefix".to_owned(), Value::Number(0u64.into()));
    fields.insert("retired_positions".to_owned(), Value::Array(Vec::new()));
    fields.insert("readers".to_owned(), Value::Object(Map::new()));
    Value::Object(fields)
}

#[allow(clippy::ptr_arg)]
fn lookup<'a>(state: &'a Value, id: &String) -> &'a Value {
    let tasks = "tasks".index_into(state).unwrap_or(&Value::Null);
    id.as_str().index_into(tasks).unwrap_or(&Value::Null)
}

fn prepare<'a, C: Caches>(
    state: &Value,
    event: &'a Value,
    caches: &mut C,
) -> Result<queue_event::Prepared<'a>, Error> {
    let prepared = match queue_event::prepare(event) {
        Ok(prepared) => prepared,
        Err(error) => return Err(Error::Prepare(error)),
    };
    let mut index = 0usize;

    while index < prepared.affected.len() {
        let id = &prepared.affected[index];
        let selected = lookup(state, id);
        if !queue_task::valid_target(prepared.kind, selected) {
            return Err(Error::Target);
        }
        caches.before(id, selected);
        index += 1;
    }

    Ok(prepared)
}

fn refresh<C: Caches>(
    state: &Value,
    affected: &[String],
    kind: &str,
    event: &Value,
    caches: &mut C,
) {
    let mut index = 0usize;

    while index < affected.len() {
        let id = &affected[index];
        caches.after(id, lookup(state, id));
        index += 1;
    }
    caches.event(kind, event);
}

#[allow(clippy::too_many_arguments)]
pub fn apply<C: Caches>(
    state: &mut Value,
    epoch: &mut String,
    order: &mut usize,
    sealed: &mut bool,
    event: &Value,
    caches: &mut C,
) -> Result<(), Error> {
    let prepared = prepare(state, event, caches)?;
    let body_outcome =
        queue_replay::body(state, epoch, order, sealed, event, prepared.kind, caches);
    let recorded = match body_outcome {
        Ok(recorded) => recorded,
        Err(error) => return Err(Error::Body(error)),
    };

    refresh(state, &prepared.affected, prepared.kind, event, caches);
    let outcome = queue_replay::finish(state, event, recorded);

    match outcome {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::Receipt(error)),
    }
}

#[derive(Debug)]
pub struct Failure {
    pub index: usize,
    pub error: Error,
}

pub fn transaction<C: Caches>(
    state: &mut Value,
    epoch: &mut String,
    order: &mut usize,
    sealed: &mut bool,
    value: &Value,
    caches: &mut C,
) -> Result<(), Failure> {
    let Value::Array(events) = value else {
        return Err(Failure {
            index: 0,
            error: Error::Transaction,
        });
    };
    let mut index = 0usize;

    while index < events.len() {
        let outcome = apply(state, epoch, order, sealed, &events[index], caches);

        if let Err(error) = outcome {
            let failure = Failure { index, error };

            return Err(failure);
        }

        index += 1;
    }

    Ok(())
}

pub trait Recovery {
    type Error;
    fn before(&mut self, index: usize, transaction: &Value) -> Result<(), Self::Error>;
}

#[derive(Debug)]
pub enum ReplayCause<E> {
    Bookkeeping(E),
    Application(Failure),
}

#[derive(Debug)]
pub struct ReplayFailure<E> {
    pub index: usize,
    pub cause: ReplayCause<E>,
}

#[allow(clippy::too_many_arguments)]
pub fn recover<C: Caches, R: Recovery>(
    state: &mut Value,
    epoch: &mut String,
    order: &mut usize,
    sealed: &mut bool,
    frames: &[Value],
    caches: &mut C,
    bookkeeping: &mut R,
) -> Result<(), ReplayFailure<R::Error>> {
    let first = if frames.is_empty() { 0 } else { 1 };
    let mut index = first;

    while index < frames.len() {
        if let Err(error) = bookkeeping.before(index, &frames[index]) {
            let failure = ReplayFailure {
                index,
                cause: ReplayCause::Bookkeeping(error),
            };

            return Err(failure);
        }

        let outcome = transaction(state, epoch, order, sealed, &frames[index], caches);

        if let Err(error) = outcome {
            let failure = ReplayFailure {
                index,
                cause: ReplayCause::Application(error),
            };

            return Err(failure);
        }

        index += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct Cache(Vec<Value>);
    impl queue_replay::Deadlines for Cache {
        fn remove(&mut self, id: &String) {
            self.0.push(json!(["remove", id]));
        }
    }
    impl Caches for Cache {
        fn before(&mut self, id: &String, task: &Value) {
            self.0.push(json!(["before", id, task["state"]]));
        }
        fn after(&mut self, id: &String, task: &Value) {
            self.0.push(json!(["after", id, task["state"]]));
        }
        fn event(&mut self, kind: &str, _: &Value) {
            self.0.push(json!(["event", kind]));
        }
    }
    #[derive(Default)]
    struct Bookkeeping {
        seen: Vec<(usize, Value)>,
        fail: Option<usize>,
    }
    impl Recovery for Bookkeeping {
        type Error = &'static str;
        fn before(&mut self, index: usize, transaction: &Value) -> Result<(), Self::Error> {
            self.seen.push((index, transaction.clone()));
            if self.fail == Some(index) {
                Err("bookkeeping")
            } else {
                Ok(())
            }
        }
    }
    fn run(state: &mut Value, events: &Value, cache: &mut Cache) -> Result<(), Failure> {
        transaction(state, &mut String::new(), &mut 0, &mut false, events, cache)
    }
    fn submitted() -> Value {
        json!({"type":"Submitted","tasks":[{"task_id":"a","input_ref":"root","max_attempts":5}],
            "request_key":"submit","request_digest":"digest","response":["a"]})
    }

    #[test]
    fn mixed_frames_use_the_actual_journal_values_in_order_and_skip_only_the_header() {
        let frames = vec![
            json!([{"type":"Run","identity":"this is not a business event"}]),
            json!([submitted(), {"type":"Leased","leases":[{"task_id":"a","attempt_id":"lease","worker_id":"worker","generation":1}]}]),
            json!([{"type":"TaskProgress","task_id":"a","input_ref":"next"},
                {"type":"Epoch","epoch":"new","revoked":["a"]}, {"type":"Sealed"}]),
        ];
        let mut journal = crate::journal_state::State::new(false);
        journal.transactions = frames.clone();
        let (mut state, mut epoch, mut order, mut sealed) = (initial(), String::new(), 0, false);
        let mut cache = Cache::default();
        let mut bookkeeping = Bookkeeping::default();
        journal
            .apply_business(
                &mut state,
                &mut epoch,
                &mut order,
                &mut sealed,
                &mut cache,
                &mut bookkeeping,
            )
            .unwrap();
        assert_eq!(
            bookkeeping.seen,
            vec![(1, frames[1].clone()), (2, frames[2].clone())]
        );
        assert_eq!(epoch, "new");
        assert_eq!(order, 1);
        assert!(sealed);
        assert_eq!(state["tasks"]["a"]["state"], "pending");
        assert_eq!(state["tasks"]["a"]["spec"]["input_ref"], "next");
        assert_eq!(state["tasks"]["a"]["failures"], 1);
        assert_eq!(state["requests"]["submit"], json!(["digest", ["a"]]));
        assert_eq!(
            cache
                .0
                .iter()
                .filter(|v| v[0] == "event")
                .cloned()
                .collect::<Vec<_>>(),
            vec![
                json!(["event", "Submitted"]),
                json!(["event", "Leased"]),
                json!(["event", "TaskProgress"]),
                json!(["event", "Epoch"]),
                json!(["event", "Sealed"])
            ]
        );
        assert_eq!(journal.transactions, frames);
        let flat = Value::Array(
            frames[1..]
                .iter()
                .flat_map(|frame| frame.as_array().unwrap().iter().cloned())
                .collect(),
        );
        let (mut direct, mut direct_epoch, mut direct_order, mut direct_sealed) =
            (initial(), String::new(), 0, false);
        transaction(
            &mut direct,
            &mut direct_epoch,
            &mut direct_order,
            &mut direct_sealed,
            &flat,
            &mut Cache::default(),
        )
        .unwrap();
        assert_eq!(
            (state, epoch, order, sealed),
            (direct, direct_epoch, direct_order, direct_sealed)
        );
    }

    #[test]
    fn admission_checks_the_whole_target_list_before_changing_business_state() {
        let mut state = initial();
        run(&mut state, &json!([submitted()]), &mut Cache::default()).unwrap();
        let before = state.clone();
        let mut cache = Cache::default();
        let failure = run(
            &mut state,
            &json!([{"type":"Expired","task_ids":["a","missing"]}]),
            &mut cache,
        )
        .unwrap_err();
        assert_eq!(failure.index, 0);
        assert!(matches!(failure.error, Error::Target));
        assert_eq!(state, before);
        assert_eq!(cache.0, vec![json!(["before", "a", "pending"])]);
        cache.0.clear();
        let failure = run(
            &mut state,
            &json!([{"type":"Expired","task_ids":["a",false]}]),
            &mut cache,
        )
        .unwrap_err();
        assert!(matches!(failure.error, Error::Prepare(_)));
        assert_eq!(state, before);
        assert!(cache.0.is_empty());
    }

    #[test]
    fn transaction_failure_keeps_the_prior_event_and_partial_failing_body() {
        let mut state = initial();
        let mut cache = Cache::default();
        let failure = run(
            &mut state,
            &json!([submitted(),
            {"type":"Leased","leases":[{"task_id":"a"}]}, {"type":"Cancelled","task_id":"a"}]),
            &mut cache,
        )
        .unwrap_err();
        assert_eq!(failure.index, 1);
        assert!(matches!(failure.error, Error::Body(_)));
        assert_eq!(state["tasks"]["a"]["state"], "leased");
        assert_eq!(state["requests"]["submit"], json!(["digest", ["a"]]));
        assert_eq!(state["readers"], json!({}));
        assert_eq!(
            cache.0,
            vec![
                json!(["before", "a", null]),
                json!(["after", "a", "pending"]),
                json!(["event", "Submitted"]),
                json!(["before", "a", "pending"])
            ]
        );
    }

    #[test]
    fn receipt_failure_occurs_after_body_and_cache_refresh_and_stops_the_transaction() {
        let mut state = initial();
        state["requests"] = Value::Null;
        let mut cache = Cache::default();
        let (mut epoch, mut order, mut sealed) = (String::new(), 0, false);
        let failure = transaction(
            &mut state,
            &mut epoch,
            &mut order,
            &mut sealed,
            &json!([submitted(), {"type":"Sealed"}]),
            &mut cache,
        )
        .unwrap_err();
        assert_eq!(failure.index, 0);
        assert!(matches!(failure.error, Error::Receipt(_)));
        assert_eq!(order, 1);
        assert!(!sealed);
        assert_eq!(state["tasks"]["a"]["state"], "pending");
        assert_eq!(state["requests"], Value::Null);
        assert_eq!(
            cache.0,
            vec![
                json!(["before", "a", null]),
                json!(["after", "a", "pending"]),
                json!(["event", "Submitted"])
            ]
        );
    }

    #[test]
    fn recovery_distinguishes_bookkeeping_failure_from_a_partial_transaction() {
        let frames = vec![
            json!([{"type":"Run"}]),
            json!([submitted()]),
            json!([{"type":"Leased","leases":[{"task_id":"a"}]}]),
            json!([{"type":"Sealed"}]),
        ];
        for fail in [Some(2), None] {
            let (mut state, mut epoch, mut order, mut sealed) =
                (initial(), String::new(), 0, false);
            let mut bookkeeping = Bookkeeping {
                fail,
                ..Default::default()
            };
            let failure = recover(
                &mut state,
                &mut epoch,
                &mut order,
                &mut sealed,
                &frames,
                &mut Cache::default(),
                &mut bookkeeping,
            )
            .unwrap_err();
            assert_eq!(failure.index, 2);
            assert_eq!(bookkeeping.seen.len(), 2);
            assert!(!sealed);
            if fail.is_some() {
                assert!(matches!(
                    failure.cause,
                    ReplayCause::Bookkeeping("bookkeeping")
                ));
                assert_eq!(state["tasks"]["a"]["state"], "pending");
            } else {
                assert!(matches!(
                    failure.cause,
                    ReplayCause::Application(Failure {
                        index: 0,
                        error: Error::Body(_)
                    })
                ));
                assert_eq!(state["tasks"]["a"]["state"], "leased");
            }
        }
        let mut state = initial();
        let before = state.clone();
        let failure = run(&mut state, &Value::Null, &mut Cache::default()).unwrap_err();
        assert!(matches!(
            failure,
            Failure {
                index: 0,
                error: Error::Transaction
            }
        ));
        assert_eq!(state, before);
    }
}
