//! Queue event target selection is invariant under successful JSON replay.
//! This does not replace the remaining task-state and scheduling-index proofs.
use crate::json_specs::{self, observe};
use crate::queue_event::{self, targets};
use serde_json::Value;
use vstd::prelude::*;

verus! {
    pub open spec fn same(left: Value, right: Value) -> bool {
        forall|depth: nat| observe(left, depth) == observe(right, depth)
    }
    pub proof fn text_equal(left: Value, right: Value)
        requires same(left, right),
        ensures json_specs::text(left) == json_specs::text(right),
    { assert(observe(left, 1) == observe(right, 1)); }

    pub proof fn field_equal(left: Value, right: Value, key: Seq<char>)
        requires same(left, right),
        ensures same(crate::request::field_or_null(left, key), crate::request::field_or_null(right, key)),
    {
        assert forall|depth: nat| observe(crate::request::field_or_null(left, key), depth)
            == observe(crate::request::field_or_null(right, key), depth) by {
            crate::catalog_json_equivalence::field_equal(left, right, key, depth);
        }
    }
    pub proof fn identity_equal(left: Value, right: Value, key: Option<Seq<char>>)
        requires same(left, right),
        ensures queue_event::identity(left, key) == queue_event::identity(right, key),
    {
        match key {
            Some(name) => {
                field_equal(left, right, name);
                text_equal(crate::request::field_or_null(left, name), crate::request::field_or_null(right, name));
            },
            None => text_equal(left, right),
        }
    }
    pub proof fn array_equal(left: Value, right: Value)
        requires same(left, right),
        ensures (json_specs::json(left) is Array) == (json_specs::json(right) is Array),
            json_specs::json(left) is Array ==> {
                let a = json_specs::json(left)->Array_0;
                let b = json_specs::json(right)->Array_0;
                a.len() == b.len() && forall|i: int| 0 <= i < a.len() ==> same(a[i], b[i])
            },
    {
        assert(observe(left, 1) == observe(right, 1));
        if json_specs::json(left) is Array {
            let a = json_specs::json(left)->Array_0;
            let b = json_specs::json(right)->Array_0;
            assert((observe(left, 1)->Array_0).len() == a.len());
            assert((observe(right, 1)->Array_0).len() == b.len());
            assert(a.len() == b.len());
            assert forall|i: int| 0 <= i < a.len() implies same(a[i], b[i]) by {
                assert forall|depth: nat| observe(a[i], depth) == observe(b[i], depth) by {
                    assert(observe(left, depth + 1) == observe(right, depth + 1));
                    assert((observe(left, depth + 1)->Array_0)[i] == observe(a[i], depth));
                    assert((observe(right, depth + 1)->Array_0)[i] == observe(b[i], depth));
                }
            }
        }
    }
    pub proof fn array_targets_equal(left: Value, right: Value, field: Seq<char>, key: Option<Seq<char>>)
        requires same(left, right),
        ensures queue_event::array_targets(left, field, key) == queue_event::array_targets(right, field, key),
    {
        field_equal(left, right, field);
        let l = crate::request::field_or_null(left, field);
        let r = crate::request::field_or_null(right, field);
        array_equal(l, r);
        if json_specs::json(l) is Array {
            let a = json_specs::json(l)->Array_0;
            let b = json_specs::json(r)->Array_0;
            assert forall|i: int| 0 <= i < a.len() implies
                queue_event::identity(a[i], key) == queue_event::identity(b[i], key) by {
                identity_equal(a[i], b[i], key);
            }
            assert(queue_event::names_valid(a, key) == queue_event::names_valid(b, key));
            assert(queue_event::names(a, key) =~= queue_event::names(b, key));
        }
    }
    pub proof fn targets_equal(left: Value, right: Value)
        requires same(left, right),
        ensures targets(left) == targets(right),
            queue_event::text(left, "type"@) == queue_event::text(right, "type"@),
    {
        identity_equal(left, right, Some("type"@));
        let kind = queue_event::text(left, "type"@);
        if kind == Some("Submitted"@) {
            array_targets_equal(left, right, "tasks"@, Some("task_id"@));
        } else if kind == Some("Leased"@) {
            array_targets_equal(left, right, "leases"@, Some("task_id"@));
        } else if kind == Some("Epoch"@) {
            array_targets_equal(left, right, "revoked"@, None);
        } else if kind == Some("Expired"@) || kind == Some("Failed"@) {
            array_targets_equal(left, right, "task_ids"@, None);
        } else if kind == Some("Cancelled"@) || kind == Some("Yielded"@) || kind == Some("TaskProgress"@) {
            identity_equal(left, right, Some("task_id"@));
        } else if kind == Some("Completed"@) {
            field_equal(left, right, "receipt"@);
            identity_equal(crate::request::field_or_null(left, "receipt"@),
                crate::request::field_or_null(right, "receipt"@), Some("task_id"@));
        }
    }
    pub proof fn parsed_targets_equal(raw: Seq<u8>, left: Value, right: Value)
        requires json_specs::parses(raw, left), json_specs::parses(raw, right),
        ensures targets(left) == targets(right),
    {
        assert forall|depth: nat| observe(left, depth) == observe(right, depth) by {
            json_specs::value_parse_consistency(raw, left, right, depth);
        }
        targets_equal(left, right);
    }
    pub proof fn preparations_equal(left: Value, right: Value,
        a: queue_event::Prepared, b: queue_event::Prepared)
        requires same(left, right), queue_event::prepared(left, a), queue_event::prepared(right, b),
        ensures a.kind@ == b.kind@, a.affected.len() == b.affected.len(),
            forall|i: int| 0 <= i < a.affected.len() ==> a.affected@[i]@ == b.affected@[i]@,
    {
        targets_equal(left, right);
        assert forall|i: int| 0 <= i < a.affected.len() implies a.affected@[i]@ == b.affected@[i]@ by {
            assert(a.affected@[i]@ == targets(left)->Some_0[i]);
            assert(b.affected@[i]@ == targets(right)->Some_0[i]);
        }
    }
    pub open spec fn transaction_targets(transaction: Value) -> Option<Seq<Option<Seq<Seq<char>>>>> {
        match json_specs::json(transaction) {
            crate::json_specs::Json::Array(events) => Some(Seq::new(events.len(), |i: int| targets(events[i]))),
            _ => None,
        }
    }
    pub proof fn transaction_targets_equal(left: Value, right: Value)
        requires same(left, right),
        ensures transaction_targets(left) == transaction_targets(right),
    {
        array_equal(left, right);
        if json_specs::json(left) is Array {
            let a = json_specs::json(left)->Array_0;
            let b = json_specs::json(right)->Array_0;
            assert forall|i: int| 0 <= i < a.len() implies targets(a[i]) == targets(b[i]) by {
                targets_equal(a[i], b[i]);
            }
            assert(transaction_targets(left)->Some_0 =~= transaction_targets(right)->Some_0);
        }
    }
}
