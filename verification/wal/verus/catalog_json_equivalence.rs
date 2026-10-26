//! Value observations to decoded catalog fields; no serde implementation is admitted here.
use crate::catalog_json::{self, Wire};
use crate::json_specs::{self, Json, Observation, field, field_in, json, observe};
use serde_json::Value;
use vstd::prelude::*;

verus! {
    pub open spec fn optional_observation(value: Option<Value>, depth: nat) -> Option<Observation> {
        match value { None => None, Some(v) => Some(observe(v, depth)) }
    }
    pub open spec fn entries_observation(items: Seq<(String, Value)>, depth: nat) -> Seq<(Seq<char>, Observation)> {
        Seq::new(items.len(), |i: int| (items[i].0@, observe(items[i].1, depth)))
    }
    pub proof fn field_in_equal(left: Seq<(String, Value)>, right: Seq<(String, Value)>, key: Seq<char>, depth: nat)
        requires entries_observation(left, depth) == entries_observation(right, depth),
        ensures optional_observation(field_in(left, key), depth) == optional_observation(field_in(right, key), depth),
        decreases left.len(),
    {
        assert(entries_observation(left, depth).len() == left.len());
        assert(entries_observation(right, depth).len() == right.len());
        if left.len() > 0 {
            assert(entries_observation(left, depth)[0] == entries_observation(right, depth)[0]);
            if left[0].0@ != key {
                assert(entries_observation(left.drop_first(), depth) =~= entries_observation(left, depth).drop_first());
                assert(entries_observation(right.drop_first(), depth) =~= entries_observation(right, depth).drop_first());
                field_in_equal(left.drop_first(), right.drop_first(), key, depth);
            }
        }
    }
    pub proof fn field_equal(left: Value, right: Value, key: Seq<char>, depth: nat)
        requires observe(left, depth + 1) == observe(right, depth + 1),
        ensures optional_observation(field(left, key), depth) == optional_observation(field(right, key), depth),
    {
        if json(left) is Object {
            assert(entries_observation(json(left)->Object_0, depth) =~= observe(left, depth + 1)->Object_0);
            assert(entries_observation(json(right)->Object_0, depth) =~= observe(right, depth + 1)->Object_0);
            field_in_equal(json(left)->Object_0, json(right)->Object_0, key, depth);
        }
    }
    pub proof fn strings_equal(left: Value, right: Value, a: Seq<String>, b: Seq<String>, depth: nat)
        requires crate::environment::string_content_contract(), depth >= 2,
            observe(left, depth) == observe(right, depth),
            catalog_json::strings_match(left, a), catalog_json::strings_match(right, b),
        ensures a == b,
    {
        reveal_with_fuel(observe, 2);
        assert((observe(left, depth)->Array_0).len() == a.len());
        assert((observe(right, depth)->Array_0).len() == b.len());
        assert(a.len() == b.len());
        assert forall|i: int| 0 <= i < a.len() implies a[i] == b[i] by {
            let av = (json(left)->Array_0)[i];
            let bv = (json(right)->Array_0)[i];
            assert((observe(left, depth)->Array_0)[i] == observe(av, (depth - 1) as nat));
            assert((observe(right, depth)->Array_0)[i] == observe(bv, (depth - 1) as nat));
            assert(a[i]@ == b[i]@);
        }
        assert(a =~= b);
    }
    pub open spec fn owner_paths(paths: Option<Vec<String>>) -> Option<Seq<String>> {
        match paths { None => None, Some(v) => Some(v@) }
    }
    pub open spec fn owners_view(items: Seq<(String, Option<Vec<String>>)>) -> Seq<(String, Option<Seq<String>>)> {
        Seq::new(items.len(), |i: int| (items[i].0, owner_paths(items[i].1)))
    }
    pub proof fn owners_equal(left: Value, right: Value, a: Seq<(String, Option<Vec<String>>)>,
        b: Seq<(String, Option<Vec<String>>)>, allow_null: bool, depth: nat)
        requires crate::environment::string_content_contract(), depth >= 3,
            observe(left, depth) == observe(right, depth),
            catalog_json::owners_match(left, a, allow_null), catalog_json::owners_match(right, b, allow_null),
        ensures owners_view(a) == owners_view(b),
    {
        reveal_with_fuel(observe, 2);
        assert((observe(left, depth)->Object_0).len() == a.len());
        assert((observe(right, depth)->Object_0).len() == b.len());
        assert(a.len() == b.len());
        assert forall|i: int| 0 <= i < a.len() implies owners_view(a)[i] == owners_view(b)[i] by {
            let av = (json(left)->Object_0)[i].1;
            let bv = (json(right)->Object_0)[i].1;
            assert((observe(left, depth)->Object_0)[i] == (observe(right, depth)->Object_0)[i]);
            assert(a[i].0@ == b[i].0@);
            if a[i].1.is_some() && b[i].1.is_some() {
                strings_equal(av, bv, a[i].1->Some_0@, b[i].1->Some_0@, (depth - 1) as nat);
            }
        }
        assert(owners_view(a) =~= owners_view(b));
    }
    pub proof fn packs_equal(left: Value, right: Value, a: Seq<(String, bool)>, b: Seq<(String, bool)>, depth: nat)
        requires crate::environment::string_content_contract(), depth >= 2,
            observe(left, depth) == observe(right, depth),
            catalog_json::packs_match(left, a), catalog_json::packs_match(right, b),
        ensures a == b,
    {
        reveal_with_fuel(observe, 2);
        assert((observe(left, depth)->Object_0).len() == a.len());
        assert((observe(right, depth)->Object_0).len() == b.len());
        assert(a.len() == b.len());
        assert forall|i: int| 0 <= i < a.len() implies a[i] == b[i] by {
            assert((observe(left, depth)->Object_0)[i] == (observe(right, depth)->Object_0)[i]);
            assert(a[i].0@ == b[i].0@);
        }
        assert(a =~= b);
    }
    pub proof fn optional_strings_equal(left: Option<Value>, right: Option<Value>, a: Seq<String>, b: Seq<String>)
        requires crate::environment::string_content_contract(),
            optional_observation(left, 3) == optional_observation(right, 3),
            catalog_json::optional_strings(left, a), catalog_json::optional_strings(right, b),
        ensures a == b,
    {
        if left.is_some() { strings_equal(left->Some_0, right->Some_0, a, b, 3); }
        else { assert(a =~= b); }
    }
    pub proof fn optional_owners_equal(left: Option<Value>, right: Option<Value>,
        a: Seq<(String, Option<Vec<String>>)>, b: Seq<(String, Option<Vec<String>>)>, allow_null: bool)
        requires crate::environment::string_content_contract(),
            optional_observation(left, 3) == optional_observation(right, 3),
            catalog_json::optional_owners(left, a, allow_null), catalog_json::optional_owners(right, b, allow_null),
        ensures owners_view(a) == owners_view(b),
    {
        if left.is_some() { owners_equal(left->Some_0, right->Some_0, a, b, allow_null, 3); }
        else { assert(owners_view(a) =~= owners_view(b)); }
    }
    pub open spec fn same_wire(a: Wire, b: Wire) -> bool {
        a.encoding == b.encoding && owners_view(a.owners@) == owners_view(b.owners@)
            && a.packs@ == b.packs@ && a.deleted@ == b.deleted@ && a.reclaimed@ == b.reclaimed@
            && owners_view(a.edges@) == owners_view(b.edges@) && a.receipts@ == b.receipts@
            && a.has_receipts == b.has_receipts && a.has_owners == b.has_owners
    }
    pub proof fn wire_equal(left: Value, right: Value, a: Wire, b: Wire)
        requires crate::environment::string_content_contract(), observe(left, 4) == observe(right, 4),
            catalog_json::wire_matches(left, a), catalog_json::wire_matches(right, b),
        ensures same_wire(a, b),
    {
        field_equal(left, right, "path_encoding"@, 3);
        field_equal(left, right, "owners"@, 3);
        field_equal(left, right, "packs"@, 3);
        field_equal(left, right, "deleted"@, 3);
        field_equal(left, right, "reclaimed"@, 3);
        field_equal(left, right, "staging_edges"@, 3);
        field_equal(left, right, "consumed_handoffs"@, 3);
        optional_owners_equal(field(left, "owners"@), field(right, "owners"@), a.owners@, b.owners@, true);
        optional_owners_equal(field(left, "staging_edges"@), field(right, "staging_edges"@), a.edges@, b.edges@, false);
        optional_strings_equal(field(left, "deleted"@), field(right, "deleted"@), a.deleted@, b.deleted@);
        optional_strings_equal(field(left, "reclaimed"@), field(right, "reclaimed"@), a.reclaimed@, b.reclaimed@);
        optional_strings_equal(field(left, "consumed_handoffs"@), field(right, "consumed_handoffs"@), a.receipts@, b.receipts@);
        if field(left, "packs"@).is_some() {
            packs_equal(field(left, "packs"@)->Some_0, field(right, "packs"@)->Some_0, a.packs@, b.packs@, 3);
        } else { assert(a.packs@ =~= b.packs@); }
    }
    pub proof fn parsed_wire_equal(raw: Seq<u8>, left: Value, right: Value, a: Wire, b: Wire)
        requires crate::environment::string_content_contract(), json_specs::parses(raw, left), json_specs::parses(raw, right),
            catalog_json::wire_matches(left, a), catalog_json::wire_matches(right, b),
        ensures same_wire(a, b),
    {
        json_specs::value_parse_consistency(raw, left, right, 4);
        wire_equal(left, right, a, b);
    }
}
