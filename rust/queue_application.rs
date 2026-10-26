//! Actual business-application quarantine and fresh recovery results.
use crate::{journal_state::State, queue_history};
use serde_json::Value;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg(straw_verus)]
verus! {
    /// First component distinguishes before/after; second records success.
    pub trait HookModel: View<V = Seq<(bool, bool)>> {}
}
#[cfg(not(straw_verus))]
pub trait HookModel {}
#[cfg(not(straw_verus))]
impl<T> HookModel for T {}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Hooks: HookModel {
    type Error;
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures final(self)@ == old(self)@.push((true, result.is_ok())),
    ))]
    fn before(&mut self) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures final(self)@ == old(self)@.push((false, result.is_ok())),
    ))]
    fn after(&mut self) -> Result<(), Self::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error<E> {
    Before(E),
    Body(queue_history::Failure),
    After(E),
}

/// Created only after the whole transaction and its final hook succeed.
/// Private fields and ownership prevent native callers from manufacturing or
/// duplicating a confirmation. Journal identity remains a separate boundary.
#[cfg_attr(straw_verus, verus_verify)]
pub(crate) struct Confirmation {
    private: (),
    #[cfg(straw_verus)]
    journal: Ghost<State>,
}

#[cfg(straw_verus)]
verus! {
    pub(crate) open spec fn guarded(before: State, after: State, poisoned: bool) -> bool {
        after == (State { poisoned, ..before })
    }
    pub(crate) closed spec fn confirms(token: Confirmation, journal: State) -> bool {
        token.journal@ == journal && journal.poisoned
    }
    pub(crate) open spec fn application_result<E>(before: queue_history::Business,
        after: queue_history::Business, events: Value, result: Result<Confirmation, Error<E>>) -> bool {
        match result {
            Ok(_) | Err(Error::After(_)) => queue_history::transaction_result(before, after, events, Ok(())),
            Err(Error::Before(_)) => after == before,
            Err(Error::Body(error)) => queue_history::transaction_result(before, after, events, Err(error)),
        }
    }
    pub(crate) open spec fn hook_result<E>(before: Seq<(bool, bool)>, after: Seq<(bool, bool)>, result: Result<Confirmation, Error<E>>) -> bool {
        after == match result {
            Err(Error::Before(_)) => before.push((true, false)),
            Err(Error::Body(_)) => before.push((true, true)),
            Err(Error::After(_)) => before.push((true, true)).push((false, false)),
            Ok(_) => before.push((true, true)).push((false, true)),
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(
    ensures guarded(*old(journal), *final(journal), true),
))]
pub(crate) fn arm(journal: &mut State) {
    journal.poisoned = true;
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires _journal.poisoned, crate::environment::string_order_contract(),
    ensures application_result((*old(state), *old(epoch), *old(order), *old(sealed)),
        (*final(state), *final(epoch), *final(order), *final(sealed)), *events, result),
        result.is_ok() ==> confirms(result->Ok_0, *_journal),
        hook_result(old(hooks)@, final(hooks)@, result),
))]
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
    #[cfg(straw_verus)]
    proof! { assert(result->Ok_0 == ()); }
    if let Err(error) = result {
        return Err(Error::Body(error));
    }
    if let Err(error) = hooks.after() {
        return Err(Error::After(error));
    }
    #[cfg(straw_verus)]
    proof! { reveal(confirms); }
    Ok(Confirmation {
        private: (),
        #[cfg(straw_verus)]
        journal: verus_exec_expr!(Ghost(*_journal)),
    })
}

#[cfg_attr(straw_verus, verus_spec(
    requires confirms(token, *old(journal)),
    ensures guarded(*old(journal), *final(journal), false),
))]
pub(crate) fn confirm(journal: &mut State, token: Confirmation) {
    let Confirmation { private: (), .. } = token;
    journal.poisoned = false;
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (method@ == "state"@ || (!journal.poisoned && !closed)),
))]
pub(crate) fn permitted(journal: &State, closed: bool, method: &str) -> bool {
    method == "state" || (!journal.poisoned && !closed)
}

#[cfg_attr(straw_verus, verus_verify)]
pub(crate) struct Restored {
    pub state: Value,
    pub epoch: String,
    pub order: usize,
    pub sealed: bool,
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub(crate) enum RestoreError<E> {
    Unavailable,
    History(queue_history::ReplayFailure<E>),
}

#[cfg(straw_verus)]
verus! {
    impl Restored {
        pub(crate) open spec fn business(self) -> queue_history::Business { (self.state, self.epoch, self.order, self.sealed) }
    }
    pub(crate) open spec fn fresh(value: queue_history::Business) -> bool {
        queue_history::initial_state(value.0) && value.1@.len() == 0 && value.2 == 0 && !value.3
    }
    pub(crate) closed spec fn rebuilt(value: Restored, frames: Seq<Value>) -> bool {
        exists|before: queue_history::Business| fresh(before)
            && queue_history::frames_applied(before, value.business(), queue_history::body_frames(frames))
    }
    pub(crate) proof fn rebuilt_fold(value: Restored, frames: Seq<Value>) -> (before: queue_history::Business)
        requires rebuilt(value, frames),
        ensures fresh(before), queue_history::folded(before, value.business(),
            queue_history::flatten(queue_history::body_frames(frames))),
    {
        reveal(rebuilt);
        let before = choose|before: queue_history::Business| fresh(before)
            && queue_history::frames_applied(before, value.business(), queue_history::body_frames(frames));
        queue_history::frames_fold(before, value.business(), queue_history::body_frames(frames));
        before
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures guarded(*old(journal), *final(journal), result.is_err()),
        old(journal).poisoned <==> result == Err(RestoreError::Unavailable),
        result.is_ok() ==> rebuilt(result->Ok_0, old(journal).transactions@),
))]
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
    #[cfg(straw_verus)]
    let before = verus_exec_expr!(Ghost(result.business()));
    let outcome = journal.apply_business(
        &mut result.state,
        &mut result.epoch,
        &mut result.order,
        &mut result.sealed,
        caches,
        bookkeeping,
    );
    #[cfg(straw_verus)]
    proof! { assert(outcome->Ok_0 == ()); }
    if let Err(error) = outcome {
        return Err(RestoreError::History(error));
    }
    #[cfg(straw_verus)]
    proof! {
        reveal(rebuilt);
        assert(fresh(before@));
        assert(queue_history::frames_applied(before@, result.business(), queue_history::body_frames(journal.transactions@)));
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
        // A new decoded State stands for the separately verified successful
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
