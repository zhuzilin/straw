//! Reader-root preservation derived from the actual compound replay contracts.
use crate::{queue_event_equivalence, queue_metadata, queue_reader, queue_state, queue_transition};
use serde_json::Value;
use vstd::prelude::*;

verus! {
    pub proof fn copied_attempt(left: Value, right: Value)
        requires queue_metadata::same_value(left, right), crate::environment::string_content_contract(),
        ensures queue_reader::attempt(left) == queue_reader::attempt(right),
    {
        queue_event_equivalence::identity_equal(left, right, Some("attempt_id"@));
        match (queue_reader::attempt(left), queue_reader::attempt(right)) {
            (Some(a), Some(b)) => { assert(a@ == b@); },
            _ => {},
        }
    }
    pub open spec fn entry(state: Value, attempt: String) -> Value {
        queue_reader::table(queue_transition::readers(state))[attempt]
    }
    pub open spec fn active(state: Value, attempt: String) -> bool {
        queue_transition::readers(state) is Object
            && queue_reader::table(queue_transition::readers(state)).contains_key(attempt)
            && queue_reader::progressable(entry(state, attempt))
    }
    pub open spec fn reader_table_preserved(before: Value, after: Value, attempt: String) -> bool {
        before is Object && after is Object
            && queue_reader::table(before).contains_key(attempt) && queue_reader::table(after).contains_key(attempt)
            && queue_reader::progressable(queue_reader::table(before)[attempt]) && queue_reader::progressable(queue_reader::table(after)[attempt])
            && (forall|i: int| 0 <= i < queue_reader::roots(queue_reader::table(before)[attempt])->Some_0.len()
                ==> i < queue_reader::roots(queue_reader::table(after)[attempt])->Some_0.len()
                    && queue_reader::roots(queue_reader::table(after)[attempt])->Some_0[i] == queue_reader::roots(queue_reader::table(before)[attempt])->Some_0[i])
            && queue_reader::table(after) == queue_reader::table(before).insert(attempt, queue_reader::table(after)[attempt])
    }
    pub open spec fn preserved(before: Value, after: Value, attempt: String) -> bool {
        reader_table_preserved(queue_transition::readers(before), queue_transition::readers(after), attempt)
    }
    pub proof fn current_preserves_roots(before: Value, after: Value, task: String, attempt: String, input: Value)
        requires crate::environment::string_content_contract(),
            queue_transition::current_progress(before, after, task, input, Ok(())),
            queue_reader::attempt(queue_transition::lease(before, task)) == Some(attempt), active(before, attempt),
        ensures preserved(before, after, attempt),
            input != Value::Null ==> queue_reader::protected(entry(after, attempt), input),
    {
        let snapshot = queue_transition::progress_snapshot(before, after, task, input, Ok(()));
        copied_attempt(snapshot, queue_transition::lease(before, task));
        if input != Value::Null {
            assert(queue_reader::selected(queue_transition::readers(before), attempt) == Some(entry(before, attempt)));
            assert(queue_reader::extended(entry(before, attempt), entry(after, attempt), input));
            queue_reader::extension_preserves_all_roots(entry(before, attempt), entry(after, attempt), input);
        }
        assert(queue_reader::table(queue_transition::readers(after)) =~= queue_reader::table(queue_transition::readers(before)).insert(attempt, entry(after, attempt)));
    }
    pub proof fn progress_preserves_roots(before: Value, after: Value, event: Value, order: u64, task: String, attempt: String)
        requires crate::environment::string_content_contract(),
            queue_transition::progress_result(before, after, event, false, order, Ok(())),
            queue_state::field_id(event, "task_id"@) == Some(task),
            queue_reader::attempt(queue_transition::lease(before, task)) == Some(attempt), active(before, attempt),
        ensures preserved(before, after, attempt),
            queue_transition::field(event, "input_ref"@) != Value::Null
                ==> queue_reader::protected(entry(after, attempt), queue_transition::field(event, "input_ref"@)),
    {
        let middle = queue_transition::progress_middle(before, after, event, order, task);
        current_preserves_roots(before, middle, task, attempt, queue_transition::field(event, "input_ref"@));
        assert(queue_transition::readers(after) == queue_transition::readers(middle));
        assert(preserved(before, after, attempt));
    }
}
