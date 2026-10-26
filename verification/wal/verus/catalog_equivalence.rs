//! Business replay equivalence, independent of collection allocation and edge iteration order.
use crate::catalog_decode::{self, Action};
use crate::catalog_state::{self, Operation, State};
use crate::catalog_json::Wire;
use vstd::prelude::*;

verus! {
    pub open spec fn step(s: State, a: Action) -> State {
        match a {
            Action::Owner(k, paths) => State { owners: s.owners.insert(k, paths), ..s },
            Action::Release(k) => State { owners: s.owners.remove(k), ..s },
            Action::Pack(p, sealed) => State { packs: s.packs.insert(p, sealed), ..s },
            Action::Mark(p) => State { deleted: s.deleted.insert(p),
                pending: if s.deleted.contains(p) { s.pending } else { s.pending.insert(p) }, ..s },
            Action::Reclaim(p) => if s.deleted.contains(p) {
                State { pending: s.pending.remove(p), packs: s.packs.remove(p), ..s }
            } else { s },
            Action::Edges(k, children) => State { edges: s.edges.insert(k, children), ..s },
            Action::Receipts(ids) => State { receipts: s.receipts.union(ids.to_set()), ..s },
        }
    }
    pub open spec fn run(s: State, actions: Seq<Action>) -> State
        decreases actions.len(),
    {
        if actions.len() == 0 { s } else { step(run(s, actions.drop_last()), actions.last()) }
    }
    pub proof fn actual_run(s: State, ops: Seq<Operation>)
        ensures catalog_state::run(s, ops) == run(s, catalog_decode::actions(ops)),
        decreases ops.len(),
    {
        catalog_state::run_unfold(s, ops);
        if ops.len() > 0 {
            actual_run(s, ops.drop_last());
            assert(catalog_decode::actions(ops).drop_last() =~= catalog_decode::actions(ops.drop_last()));
        }
    }
    pub proof fn run_concat(s: State, first: Seq<Action>, last: Seq<Action>)
        ensures run(s, first + last) == run(run(s, first), last),
        decreases last.len(),
    {
        if last.len() > 0 {
            run_concat(s, first, last.drop_last());
            assert((first + last).drop_last() == first + last.drop_last());
        } else { assert(first + last == first); }
    }
    pub open spec fn overlay(base: Map<String, Seq<String>>, added: Map<String, Seq<String>>) -> Map<String, Seq<String>> {
        Map::new(base.dom().union(added.dom()), |key: String|
            if added.contains_key(key) { added[key] } else { base[key] })
    }
    pub open spec fn covered(graph: Map<String, Vec<String>>, items: Seq<(String, Vec<String>)>) -> Set<String> {
        graph.dom().filter(|key: String| exists|i: int| 0 <= i < items.len() && items[i].0 == key)
    }
    pub proof fn edges_fold(s: State, items: Seq<(String, Vec<String>)>, graph: Map<String, Vec<String>>)
        requires items.to_set().subset_of(graph.kv_pairs()),
        ensures run(s, catalog_decode::edge_actions(items)) == (State {
            edges: overlay(s.edges, Map::new(
                covered(graph, items),
                |key: String| graph[key]@)), ..s }),
        decreases items.len(),
    {
        if items.len() > 0 {
            let prefix = items.drop_last();
            assert(prefix.to_set().subset_of(graph.kv_pairs())) by {
                assert forall|pair: (String, Vec<String>)| prefix.to_set().contains(pair)
                    implies graph.kv_pairs().contains(pair) by {
                    let i = choose|i: int| 0 <= i < prefix.len() && prefix[i] == pair;
                    assert(items[i] == pair);
                    assert(items.to_set().contains(pair));
                }
            }
            edges_fold(s, prefix, graph);
            assert(catalog_decode::edge_actions(items).drop_last() =~= catalog_decode::edge_actions(prefix));
            assert(items.to_set().contains(items.last()));
            assert(graph.contains_key(items.last().0));
            assert forall|key: String| #[trigger] covered(graph, items).contains(key)
                == (covered(graph, prefix).contains(key) || key == items.last().0) by {
                if exists|i: int| 0 <= i < items.len() && items[i].0 == key {
                    let i = choose|i: int| 0 <= i < items.len() && items[i].0 == key;
                    if i < prefix.len() { assert(prefix[i].0 == key); }
                } else if exists|i: int| 0 <= i < prefix.len() && prefix[i].0 == key {
                    let i = choose|i: int| 0 <= i < prefix.len() && prefix[i].0 == key;
                    assert(items[i].0 == key);
                } else if key == items.last().0 { assert(items[items.len() - 1].0 == key); }
            }
            assert(run(s, catalog_decode::edge_actions(items)).edges =~= overlay(s.edges, Map::new(
                covered(graph, items),
                |key: String| graph[key]@)));
        } else {
            assert(covered(graph, items) =~= Set::<String>::empty());
            assert(overlay(s.edges, Map::new(Set::<String>::empty(), |key: String| graph[key]@)) =~= s.edges);
        }
    }
    pub proof fn edge_effect(s: State, ops: Seq<Operation>, graph: Map<String, Vec<String>>)
        requires catalog_decode::edge_output(ops, graph),
        ensures run(s, catalog_decode::actions(ops)) == (State {
            edges: overlay(s.edges, catalog_state::edge_view(graph)), ..s }),
    {
        let items = choose|items: Seq<(String, Vec<String>)>| items.to_set() == graph.kv_pairs()
            && items.no_duplicates() && catalog_decode::actions(ops) == catalog_decode::edge_actions(items);
        edges_fold(s, items, graph);
        assert forall|key: String| #[trigger] covered(graph, items).contains(key)
            == graph.contains_key(key) by {
            if graph.contains_key(key) {
                assert(items.to_set().contains((key, graph[key])));
                let i = choose|i: int| 0 <= i < items.len() && items[i] == (key, graph[key]);
                assert(items[i].0 == key);
            } else if exists|i: int| 0 <= i < items.len() && items[i].0 == key {
                let i = choose|i: int| 0 <= i < items.len() && items[i].0 == key;
                assert(items.to_set().contains(items[i]));
            }
        }
        assert(Map::new(covered(graph, items),
            |key: String| graph[key]@) =~= catalog_state::edge_view(graph));
    }
    pub open spec fn ordered_actions(ns: Map<String, String>, wire: Wire) -> Seq<Action> {
        catalog_decode::owner_actions(ns, wire.owners@, catalog_decode::wire_encoding(wire))
            + catalog_decode::pack_actions(ns, wire.packs@, catalog_decode::wire_encoding(wire))
            + catalog_decode::deletion_actions(ns, wire.deleted@, catalog_decode::wire_encoding(wire), false)
            + catalog_decode::deletion_actions(ns, wire.reclaimed@, catalog_decode::wire_encoding(wire), true)
            + seq![Action::Receipts(wire.receipts@)]
    }
    pub open spec fn effect(s: State, ns: Map<String, String>, wire: Wire) -> State {
        run(State { edges: overlay(s.edges, catalog_state::edge_view(catalog_decode::edge_map(wire.edges@))), ..s },
            ordered_actions(ns, wire))
    }
    pub proof fn meaning_effect(s: State, ns: Map<String, String>, wire: Wire, ops: Seq<Operation>)
        requires catalog_decode::meaning(ns, wire, ops),
        ensures catalog_state::run(s, ops) == effect(s, ns, wire),
    {
        let prefix = choose|prefix: Seq<Operation>| catalog_decode::edge_output(prefix, catalog_decode::edge_map(wire.edges@))
            && catalog_decode::actions(ops) == catalog_decode::actions(prefix)
                + catalog_decode::owner_actions(ns, wire.owners@, catalog_decode::wire_encoding(wire))
                + catalog_decode::pack_actions(ns, wire.packs@, catalog_decode::wire_encoding(wire))
                + catalog_decode::deletion_actions(ns, wire.deleted@, catalog_decode::wire_encoding(wire), false)
                + catalog_decode::deletion_actions(ns, wire.reclaimed@, catalog_decode::wire_encoding(wire), true)
                + seq![Action::Receipts(wire.receipts@)];
        assert(catalog_decode::actions(ops) =~= catalog_decode::actions(prefix) + ordered_actions(ns, wire));
        actual_run(s, ops);
        run_concat(s, catalog_decode::actions(prefix), ordered_actions(ns, wire));
        edge_effect(s, prefix, catalog_decode::edge_map(wire.edges@));
    }
    pub proof fn same_wire_same_state(s: State, ns: Map<String, String>, wire: Wire,
        left: Seq<Operation>, right: Seq<Operation>)
        requires catalog_decode::meaning(ns, wire, left), catalog_decode::meaning(ns, wire, right),
        ensures catalog_state::run(s, left) == catalog_state::run(s, right),
    {
        meaning_effect(s, ns, wire, left);
        meaning_effect(s, ns, wire, right);
    }

    pub proof fn edge_map_equal(left: Seq<(String, Option<Vec<String>>)>, right: Seq<(String, Option<Vec<String>>)>)
        requires crate::catalog_json_equivalence::owners_view(left) == crate::catalog_json_equivalence::owners_view(right),
        ensures catalog_state::edge_view(catalog_decode::edge_map(left))
            == catalog_state::edge_view(catalog_decode::edge_map(right)),
        decreases left.len(),
    {
        assert(crate::catalog_json_equivalence::owners_view(left).len() == left.len());
        assert(crate::catalog_json_equivalence::owners_view(right).len() == right.len());
        if left.len() > 0 {
            assert(crate::catalog_json_equivalence::owners_view(left).last() == crate::catalog_json_equivalence::owners_view(right).last());
            assert(left.last().0 == right.last().0);
            assert(crate::catalog_json_equivalence::owner_paths(left.last().1)
                == crate::catalog_json_equivalence::owner_paths(right.last().1));
            assert(crate::catalog_json_equivalence::owners_view(left.drop_last()) =~= crate::catalog_json_equivalence::owners_view(left).drop_last());
            assert(crate::catalog_json_equivalence::owners_view(right.drop_last()) =~= crate::catalog_json_equivalence::owners_view(right).drop_last());
            edge_map_equal(left.drop_last(), right.drop_last());
            if left.last().1.is_some() {
                let lv = left.last().1->Some_0;
                let rv = right.last().1->Some_0;
                assert(lv@ == rv@);
                assert(catalog_state::edge_view(catalog_decode::edge_map(left)) =~=
                    catalog_state::edge_view(catalog_decode::edge_map(left.drop_last())).insert(left.last().0, lv@));
                assert(catalog_state::edge_view(catalog_decode::edge_map(right)) =~=
                    catalog_state::edge_view(catalog_decode::edge_map(right.drop_last())).insert(right.last().0, rv@));
            }
            assert(catalog_state::edge_view(catalog_decode::edge_map(left)) =~= catalog_state::edge_view(catalog_decode::edge_map(right)));
        }
    }
    pub proof fn wire_effect_equal(s: State, ns: Map<String, String>, left: Wire, right: Wire)
        requires crate::catalog_json_equivalence::same_wire(left, right),
        ensures effect(s, ns, left) == effect(s, ns, right),
    {
        edge_map_equal(left.edges@, right.edges@);
        assert(crate::catalog_json_equivalence::owners_view(left.owners@).len() == left.owners@.len());
        assert(crate::catalog_json_equivalence::owners_view(right.owners@).len() == right.owners@.len());
        assert(catalog_decode::owner_actions(ns, left.owners@, catalog_decode::wire_encoding(left))
            =~= catalog_decode::owner_actions(ns, right.owners@, catalog_decode::wire_encoding(right))) by {
            assert forall|i: int| 0 <= i < left.owners@.len() implies
                catalog_decode::owner_actions(ns, left.owners@, catalog_decode::wire_encoding(left))[i]
                    == catalog_decode::owner_actions(ns, right.owners@, catalog_decode::wire_encoding(right))[i] by {
                assert(crate::catalog_json_equivalence::owners_view(left.owners@)[i]
                    == crate::catalog_json_equivalence::owners_view(right.owners@)[i]);
                assert(left.owners@[i].0 == right.owners@[i].0);
                assert(crate::catalog_json_equivalence::owner_paths(left.owners@[i].1)
                    == crate::catalog_json_equivalence::owner_paths(right.owners@[i].1));
            }
            assert(crate::catalog_json_equivalence::owners_view(left.owners@).len() == left.owners@.len());
            assert(crate::catalog_json_equivalence::owners_view(right.owners@).len() == right.owners@.len());
        }
    }
    pub proof fn raw_effect_equal(context: crate::catalog_frame::Context, raw: Seq<u8>, s: State,
        known_left: Map<String, Seq<String>>, known_right: Map<String, Seq<String>>,
        left: Seq<Operation>, right: Seq<Operation>)
        requires crate::environment::string_content_contract(),
            crate::catalog_frame::meaning(context, raw, known_left, left),
            crate::catalog_frame::meaning(context, raw, known_right, right),
        ensures catalog_state::run(s, left) == catalog_state::run(s, right),
    {
        let (lv, lw, lk) = choose|value: serde_json::Value, wire: Wire, keys: Map<String, Vec<String>>|
            crate::json_specs::parses(raw, value) && crate::catalog_frame::same_run(value, context.run)
                && crate::catalog_json::schema(value) && crate::catalog_json::wire_matches(value, wire)
                && catalog_state::edge_view(keys) == known_left && catalog_decode::admitted(wire, keys)
                && catalog_decode::resolved_wire(context.namespace, wire)
                && catalog_decode::meaning(context.namespace, wire, left);
        let (rv, rw, rk) = choose|value: serde_json::Value, wire: Wire, keys: Map<String, Vec<String>>|
            crate::json_specs::parses(raw, value) && crate::catalog_frame::same_run(value, context.run)
                && crate::catalog_json::schema(value) && crate::catalog_json::wire_matches(value, wire)
                && catalog_state::edge_view(keys) == known_right && catalog_decode::admitted(wire, keys)
                && catalog_decode::resolved_wire(context.namespace, wire)
                && catalog_decode::meaning(context.namespace, wire, right);
        crate::catalog_json_equivalence::parsed_wire_equal(raw, lv, rv, lw, rw);
        meaning_effect(s, context.namespace, lw, left);
        meaning_effect(s, context.namespace, rw, right);
        wire_effect_equal(s, context.namespace, lw, rw);
    }
    pub proof fn raw_history_state_equal(context: crate::catalog_frame::Context,
        left: Seq<crate::catalog_log::Record>, right: Seq<crate::catalog_log::Record>)
        requires crate::environment::string_content_contract(),
            crate::catalog_target::interpreted(context, left), crate::catalog_target::interpreted(context, right),
            crate::catalog_log::payloads(left) == crate::catalog_log::payloads(right),
        ensures crate::catalog_log::history_state(left) == crate::catalog_log::history_state(right),
        decreases left.len(),
    {
        assert(crate::catalog_log::payloads(left).len() == left.len());
        assert(crate::catalog_log::payloads(right).len() == right.len());
        crate::catalog_log::history_unfold(left);
        crate::catalog_log::history_unfold(right);
        if left.len() > 0 {
            crate::catalog_target::interpreted_prefix(context, left);
            crate::catalog_target::interpreted_prefix(context, right);
            assert(crate::catalog_log::payloads(left.drop_last()) =~= crate::catalog_log::payloads(left).drop_last());
            assert(crate::catalog_log::payloads(right.drop_last()) =~= crate::catalog_log::payloads(right).drop_last());
            raw_history_state_equal(context, left.drop_last(), right.drop_last());
            crate::catalog_target::interpreted_at(context, left, (left.len() - 1) as nat);
            crate::catalog_target::interpreted_at(context, right, (right.len() - 1) as nat);
            assert(left.take(left.len() - 1) == left.drop_last());
            assert(right.take(right.len() - 1) == right.drop_last());
            assert(crate::catalog_log::payloads(left).last() == left.last().payload);
            assert(crate::catalog_log::payloads(right).last() == right.last().payload);
            raw_effect_equal(context, left.last().payload, crate::catalog_log::history_state(left.drop_last()),
                crate::catalog_log::history_state(left.drop_last()).edges,
                crate::catalog_log::history_state(right.drop_last()).edges,
                left.last().operations, right.last().operations);
        }
    }
    /// Applies to the real completed LogState produced by append and recovery.
    /// The filesystem/recovery contract separately determines which raw prefix survives.
    pub proof fn completed_states_equal(context: crate::catalog_frame::Context,
        left: crate::catalog_log::State, right: crate::catalog_log::State)
        requires crate::environment::string_content_contract(),
            crate::catalog_log::valid(left), crate::catalog_log::valid(right), left.complete, right.complete,
            crate::catalog_target::interpreted(context, left.history), crate::catalog_target::interpreted(context, right.history),
            crate::catalog_log::payloads(left.history) == crate::catalog_log::payloads(right.history),
        ensures left.indexes == right.indexes, left.offset == right.offset, left.sequence == right.sequence,
    {
        crate::catalog_log::complete_history(left);
        crate::catalog_log::complete_history(right);
        raw_history_state_equal(context, left.history, right.history);
        crate::catalog_log::payload_bytes(left.history);
        crate::catalog_log::payload_bytes(right.history);
        assert(crate::catalog_log::payloads(left.history).len() == left.history.len());
        assert(crate::catalog_log::payloads(right.history).len() == right.history.len());
    }
}
