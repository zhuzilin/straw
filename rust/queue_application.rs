//! Actual business-application quarantine and fresh recovery results.
use crate::{journal_state::State, queue_history};
use serde_json::Value;

pub trait Hooks {
    type Error;

    fn before(&mut self) -> Result<(), Self::Error>;

    fn after(&mut self) -> Result<(), Self::Error>;
}

#[derive(Debug)]
pub enum Error<E> {
    Before(E),
    Body(queue_history::Failure),
    After(E),
}

/// Created only after the whole transaction and its final hook succeed.
/// Private fields and ownership prevent native callers from manufacturing or
/// duplicating a confirmation. Journal identity remains a separate boundary.
pub(crate) struct Confirmation {
    private: (),
}

pub(crate) fn arm(journal: &mut State) {
    journal.poisoned = true;
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply<C: queue_history::Caches, H: Hooks>(
    _journal: &State,
    state: &mut Value,
    epoch: &mut String,
    order: &mut usize,
    sealed: &mut bool,
    events: &Value,
    caches: &mut C,
    hooks: &mut H,
) -> Result<Confirmation, Error<H::Error>> {
    if let Err(error) = hooks.before() {
        return Err(Error::Before(error));
    }
    let result = queue_history::transaction(state, epoch, order, sealed, events, caches);

    if let Err(error) = result {
        return Err(Error::Body(error));
    }
    if let Err(error) = hooks.after() {
        return Err(Error::After(error));
    }

    Ok(Confirmation { private: () })
}

pub(crate) fn confirm(journal: &mut State, token: Confirmation) {
    let Confirmation { private: (), .. } = token;
    journal.poisoned = false;
}

pub(crate) fn permitted(journal: &State, closed: bool, method: &str) -> bool {
    method == "state" || (!journal.poisoned && !closed)
}

pub(crate) struct Restored {
    pub state: Value,
    pub epoch: String,
    pub order: usize,
    pub sealed: bool,
}

#[derive(Debug)]
pub(crate) enum RestoreError<E> {
    Unavailable,
    History(queue_history::ReplayFailure<E>),
}

pub(crate) fn restore<C: queue_history::Caches, R: queue_history::Recovery>(
    journal: &mut State,
    caches: &mut C,
    bookkeeping: &mut R,
) -> Result<Restored, RestoreError<R::Error>> {
    if journal.poisoned {
        return Err(RestoreError::Unavailable);
    }
    arm(journal);
    let mut result = Restored {
        state: queue_history::initial(),
        epoch: String::new(),
        order: 0,
        sealed: false,
    };

    let outcome = journal.apply_business(
        &mut result.state,
        &mut result.epoch,
        &mut result.order,
        &mut result.sealed,
        caches,
        bookkeeping,
    );

    if let Err(error) = outcome {
        return Err(RestoreError::History(error));
    }

    journal.poisoned = false;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[derive(Default)]
    struct Cache;
    impl crate::queue_replay::Deadlines for Cache {
        fn remove(&mut self, _: &String) {}
    }
    impl queue_history::Caches for Cache {
        fn before(&mut self, _: &String, _: &Value) {}
        fn after(&mut self, _: &String, _: &Value) {}
        fn event(&mut self, _: &str, _: &Value) {}
    }
    struct Callback {
        fail: Option<&'static str>,
        seen: Vec<&'static str>,
    }
    impl Hooks for Callback {
        type Error = &'static str;
        fn before(&mut self) -> Result<(), Self::Error> {
            self.seen.push("before");
            if self.fail == Some("before") {
                Err("before")
            } else {
                Ok(())
            }
        }
        fn after(&mut self) -> Result<(), Self::Error> {
            self.seen.push("after");
            if self.fail == Some("after") {
                Err("after")
            } else {
                Ok(())
            }
        }
    }
    struct Bookkeeping(bool);
    impl queue_history::Recovery for Bookkeeping {
        type Error = ();
        fn before(&mut self, _: usize, _: &Value) -> Result<(), Self::Error> {
            if self.0 { Err(()) } else { Ok(()) }
        }
    }
    fn events() -> Value {
        json!([{"type":"Submitted","tasks":[{"task_id":"a","input_ref":"root","max_attempts":3}]}])
    }

    #[test]
    fn both_callback_failure_positions_leave_the_journal_quarantined() {
        for fail in [Some("before"), Some("after"), None] {
            let mut journal = State::new(false);
            journal.sequence = 7;
            let (mut state, mut epoch, mut order, mut sealed) =
                (queue_history::initial(), String::new(), 0, false);
            let mut hooks = Callback {
                fail,
                seen: Vec::new(),
            };
            arm(&mut journal);
            let outcome = apply(
                &journal,
                &mut state,
                &mut epoch,
                &mut order,
                &mut sealed,
                &events(),
                &mut Cache,
                &mut hooks,
            );
            assert!(journal.poisoned);
            assert_eq!(journal.sequence, 7);
            assert!(!permitted(&journal, false, "collect_garbage"));
            assert!(!permitted(&journal, false, "submit_tasks"));
            assert!(permitted(&journal, false, "state"));
            match (fail, outcome) {
                (Some("before"), Err(Error::Before("before"))) => {
                    assert_eq!(order, 0);
                    assert_eq!(state["tasks"], json!({}));
                    assert_eq!(hooks.seen, vec!["before"]);
                }
                (Some("after"), Err(Error::After("after"))) => {
                    assert_eq!(order, 1);
                    assert_eq!(state["tasks"]["a"]["state"], "pending");
                    assert_eq!(hooks.seen, vec!["before", "after"]);
                }
                (None, Ok(token)) => {
                    assert_eq!(order, 1);
                    confirm(&mut journal, token);
                    assert!(!journal.poisoned);
                    assert!(permitted(&journal, false, "submit_tasks"));
                    assert!(!permitted(&journal, true, "submit_tasks"));
                }
                _ => panic!("wrong application outcome"),
            }
        }
    }

    #[test]
    fn partial_body_failure_never_reaches_the_final_hook_or_confirmation() {
        let mut journal = State::new(false);
        let (mut state, mut epoch, mut order, mut sealed) =
            (queue_history::initial(), String::new(), 0, false);
        let mut transaction = events();
        transaction
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"Leased","leases":[{"task_id":"a"}]}));
        let mut hooks = Callback {
            fail: None,
            seen: Vec::new(),
        };
        arm(&mut journal);
        let result = apply(
            &journal,
            &mut state,
            &mut epoch,
            &mut order,
            &mut sealed,
            &transaction,
            &mut Cache,
            &mut hooks,
        );
        assert!(matches!(
            result,
            Err(Error::Body(queue_history::Failure { index: 1, .. }))
        ));
        assert!(journal.poisoned);
        assert_eq!(hooks.seen, vec!["before"]);
        assert_eq!(order, 1);
        assert_eq!(state["tasks"]["a"]["state"], "leased");
        assert_eq!(state["readers"], json!({}));
    }

    #[test]
    fn failed_restore_returns_no_business_state_and_requires_fresh_physical_recovery() {
        let frames = vec![
            json!([{"type":"Run"}]),
            events(),
            json!([{"type":"Sealed"}]),
        ];
        let mut journal = State::new(false);
        journal.transactions = frames.clone();
        assert!(matches!(
            restore(&mut journal, &mut Cache, &mut Bookkeeping(true)),
            Err(RestoreError::History(_))
        ));
        assert!(journal.poisoned);
        assert!(matches!(
            restore(&mut journal, &mut Cache, &mut Bookkeeping(false)),
            Err(RestoreError::Unavailable)
        ));
        // A new decoded State stands for the successful
        // physical recovery; the integration tests use real Journal::open.
        let mut fresh = State::new(false);
        fresh.transactions = frames;
        let restored = restore(&mut fresh, &mut Cache, &mut Bookkeeping(false)).unwrap();
        assert!(!fresh.poisoned);
        assert_eq!(restored.order, 1);
        assert!(restored.sealed);
        assert!(restored.epoch.is_empty());
        assert_eq!(restored.state["tasks"]["a"]["pending_order"], 0);
        assert_eq!(restored.state["tasks"].as_object().unwrap().len(), 1);
    }
}
