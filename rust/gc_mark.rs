//! Discover every unowned sealed pack and durably mark the same identities.
//! Input paths come from recovered catalog indexes, already in resolved form.
use crate::catalog_decode::Resolver;
use crate::catalog_log::LogState;
use serde_json::Value;
use std::collections::HashSet;
#[cfg(straw_verus)]
use vstd::{prelude::*, std_specs::iter::IteratorSpec};

#[cfg_attr(straw_verus, verus_verify)]
pub struct Marked {
    pub live: HashSet<String>,
    pub targets: Vec<String>,
}

#[cfg(straw_verus)]
verus! {
    use crate::catalog_state::{State, Operation};
    use crate::json_specs::{field, json, Json};
    pub open spec fn names(items: Seq<Value>) -> Seq<String> {
        Seq::new(items.len(), |i: int| match items[i] { Value::String(s) => s, _ => arbitrary() })
    }
    pub open spec fn string_values(items: Seq<Value>, paths: Set<String>) -> bool {
        (forall|i: int| 0 <= i < items.len() ==> items[i] is String)
            && names(items).to_set() == paths
    }
    pub open spec fn mark_event(value: Value, paths: Set<String>) -> bool {
        json(value) is Object
            && field(value, "deleted"@).is_some()
            && field(value, "deleted"@)->Some_0 is Array
            && string_values(json(field(value, "deleted"@)->Some_0)->Array_0, paths)
            && field(value, "path_encoding"@).is_some()
            && json(field(value, "path_encoding"@)->Some_0) == Json::Number(Some(1))
            && forall|key: Seq<char>| key != "deleted"@ && key != "path_encoding"@
                ==> field(value, key).is_none()
    }
    pub open spec fn marked(s: State, paths: Set<String>) -> State {
        State { deleted: s.deleted.union(paths),
            pending: s.pending.union(paths.difference(s.deleted)), ..s }
    }
    pub proof fn mark_one(s: State, paths: Set<String>, p: String)
        ensures crate::catalog_state::step(marked(s, paths), Operation::Mark(p)) == marked(s, paths.insert(p)),
    {
        assert(s.deleted.union(paths).insert(p) =~= s.deleted.union(paths.insert(p)));
        assert(s.pending.union(paths.insert(p).difference(s.deleted)) =~=
            if s.deleted.union(paths).contains(p) { s.pending.union(paths.difference(s.deleted)) }
            else { s.pending.union(paths.difference(s.deleted)).insert(p) });
    }
    pub proof fn mark_fold(s: State, ops: Seq<Operation>, paths: Seq<String>)
        requires ops.len() == paths.len(),
            forall|i: int| 0 <= i < ops.len() ==> ops[i] == Operation::Mark(paths[i]),
        ensures crate::catalog_state::run(s, ops) == marked(s, paths.to_set()),
        decreases ops.len(),
    {
        crate::catalog_state::run_unfold(s, ops);
        if ops.len() > 0 {
            mark_fold(s, ops.drop_last(), paths.drop_last());
            paths.drop_last().lemma_push_to_set_commute(paths.last());
            assert(paths.drop_last().push(paths.last()) == paths);
            mark_one(s, paths.drop_last().to_set(), paths.last());
        } else {
            assert(paths.to_set() =~= Set::<String>::empty());
            assert(s.deleted.union(paths.to_set()) =~= s.deleted);
            assert(s.pending.union(paths.to_set().difference(s.deleted)) =~= s.pending);
        }
    }
    pub open spec fn eligible_paths(s: State) -> Set<String> {
        s.packs.dom().filter(|p: String| crate::catalog_state::eligible(s, p))
    }
    pub proof fn wire_marks(ns: Map<String, String>, value: Value, wire: crate::catalog_json::Wire,
        ops: Seq<Operation>, paths: Set<String>, s: State)
        requires crate::environment::string_content_contract(),
            field(value, "deleted"@).is_some(), field(value, "deleted"@)->Some_0 is Array,
            string_values(json(field(value, "deleted"@)->Some_0)->Array_0, paths),
            field(value, "path_encoding"@).is_some(),
            json(field(value, "path_encoding"@)->Some_0) == Json::Number(Some(1)),
            field(value, "owners"@).is_none(), field(value, "packs"@).is_none(),
            field(value, "reclaimed"@).is_none(), field(value, "staging_edges"@).is_none(),
            field(value, "consumed_handoffs"@).is_none(),
            crate::catalog_json::wire_matches(value, wire), crate::catalog_decode::meaning(ns, wire, ops),
        ensures crate::catalog_state::run(s, ops) == marked(s, paths),
    {
        use crate::catalog_decode::{actions, Action, edge_output, edge_map, edge_actions};
        let items = json(field(value, "deleted"@)->Some_0)->Array_0;
        assert forall|i: int| 0 <= i < items.len() implies wire.deleted@[i] == names(items)[i] by {
            assert(json(items[i]) == Json::Text(wire.deleted@[i]@));
        }
        assert(wire.deleted@ =~= names(items));
        assert(wire.owners@.len() == 0 && wire.packs@.len() == 0 && wire.reclaimed@.len() == 0
            && wire.edges@.len() == 0 && wire.receipts@.len() == 0 && wire.encoding == Some(1));
        let prefix = choose|prefix: Seq<Operation>| edge_output(prefix, edge_map(wire.edges@))
            && actions(ops) == actions(prefix)
                + crate::catalog_decode::owner_actions(ns, wire.owners@, crate::catalog_decode::wire_encoding(wire))
                + crate::catalog_decode::pack_actions(ns, wire.packs@, crate::catalog_decode::wire_encoding(wire))
                + crate::catalog_decode::deletion_actions(ns, wire.deleted@, crate::catalog_decode::wire_encoding(wire), false)
                + crate::catalog_decode::deletion_actions(ns, wire.reclaimed@, crate::catalog_decode::wire_encoding(wire), true)
                + seq![Action::Receipts(wire.receipts@)];
        let edges = choose|edges: Seq<(String, Vec<String>)>| edges.to_set() == edge_map(wire.edges@).kv_pairs()
            && edges.no_duplicates() && actions(prefix) == edge_actions(edges);
        assert(edge_map(wire.edges@) == Map::<String, Vec<String>>::empty());
        if edges.len() > 0 { assert(edges.to_set().contains(edges[0])); }
        assert(edges.len() == 0);
        assert(prefix.len() == 0);
        assert(ops.len() == wire.deleted@.len() + 1);
        assert forall|i: int| 0 <= i < ops.len() - 1 implies ops.drop_last()[i] == Operation::Mark(wire.deleted@[i]) by {
            assert(actions(ops)[i] == Action::Mark(wire.deleted@[i]));
        }
        mark_fold(s, ops.drop_last(), wire.deleted@);
        crate::catalog_state::run_unfold(s, ops);
        assert(actions(ops).last() == Action::Receipts(wire.receipts@));
        assert(crate::catalog_state::run(s, ops).receipts =~= s.receipts);
    }
    pub proof fn append_marks(context: crate::catalog_frame::Context, before: crate::catalog_log::State,
        after: crate::catalog_log::State, input: Value, paths: Set<String>)
        requires crate::environment::string_content_contract(),
            crate::catalog_log::valid(before), crate::catalog_log::ready(before),
            crate::catalog_log::valid(after), crate::catalog_log::ready(after),
            after.history.len() == before.history.len() + 1, after.history.drop_last() == before.history,
            mark_event(input, paths), crate::catalog_append::intended(context, input, before.indexes.edges,
                after.history.last().payload, after.history.last().operations),
        ensures after.indexes == marked(before.indexes, paths),
    {
        let (value, wire, keys) = choose|value: Value, wire: crate::catalog_json::Wire, keys: Map<String, Vec<String>>|
            crate::catalog_append::binds_run(input, value, context.run)
                && crate::json_specs::parses(after.history.last().payload, value)
                && crate::catalog_json::schema(value) && crate::catalog_json::wire_matches(value, wire)
                && crate::catalog_state::edge_view(keys) == before.indexes.edges
                && crate::catalog_decode::admitted(wire, keys)
                && crate::catalog_decode::resolved_wire(context.namespace, wire)
                && crate::catalog_decode::meaning(context.namespace, wire, after.history.last().operations);
        wire_marks(context.namespace, value, wire, after.history.last().operations, paths, before.indexes);
        crate::catalog_log::complete_history(before);
        crate::catalog_log::complete_history(after);
        crate::catalog_log::history_unfold(after.history);
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures result@.to_set() == paths@,
))]
fn snapshot(paths: &HashSet<String>) -> Vec<String> {
    let mut result = Vec::with_capacity(paths.len());
    let mut entries = paths.iter();
    #[cfg_attr(straw_verus, verus_spec(
        invariant entries.decrease().is_some(),
            result@.to_set().union(entries.remaining().unref().to_set()) == paths@,
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        #[cfg(straw_verus)]
        proof! {
            if entries.remaining().len() > 0 {
                crate::catalog_state::pop_set(entries.remaining());
            }
        }
        let Some(path) = entries.next() else {
            return result;
        };
        #[cfg(straw_verus)]
        proof! { result@.lemma_push_to_set_commute(*path); }
        result.push(path.clone());
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures mark_event(result, paths@.to_set()),
))]
fn event(paths: &[String]) -> Value {
    let mut values = Vec::with_capacity(paths.len());
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= paths.len(), values.len() == i,
            forall|j: int| 0 <= j < i ==> values@[j] == Value::String(paths@[j]),
        decreases paths.len() - i,
    ))]
    while i < paths.len() {
        values.push(Value::String(paths[i].clone()));
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! { assert(names(values@) =~= paths@); }
    let mut object = serde_json::Map::new();
    #[cfg(straw_verus)]
    proof! {
        assert forall|key: Seq<char>| crate::json_specs::field_in(crate::json_specs::entries(object), key).is_none() by {};
    }
    object.insert("deleted".to_owned(), Value::Array(values));
    object.insert("path_encoding".to_owned(), Value::Number(1u64.into()));
    Value::Object(object)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(resolver).valid(), crate::catalog_log::valid(old(log)@), crate::catalog_log::ready(old(log)@),
        old(device).valid(), !old(device)@.buffered,
        crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
        crate::environment::string_content_contract(),
        crate::catalog_target::interpreted(
            crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ }, old(log)@.history),
    ensures crate::catalog_log::at_boundary(old(log)@,old(device)@) && crate::catalog_log::ready(final(log)@)
            ==> crate::catalog_log::at_boundary(final(log)@,final(device)@),
        final(device).loan() == old(device).loan(), final(resolver).valid(), crate::catalog_log::valid(final(log)@), final(resolver)@ == old(resolver)@,
        final(device).valid(), final(device)@.identity == old(device)@.identity, !final(device)@.buffered,
        crate::catalog_target::interpreted(
            crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ }, final(log)@.history),
        result.is_err() ==> final(log)@.history == old(log)@.history && final(log)@.indexes == old(log)@.indexes,
        match result {
            Err(crate::catalog_append::Error::Append(crate::catalog_log::AppendFailure::Write(_))) =>
                !crate::catalog_log::ready(final(log)@),
            _ => true,
        },
        result.is_ok() ==> crate::catalog_log::ready(final(log)@)
            && final(log)@.indexes == marked(old(log)@.indexes, eligible_paths(old(log)@.indexes))
            && result->Ok_0.targets@.to_set() == final(log)@.indexes.pending
            && result->Ok_0.targets@.to_set().subset_of(final(log)@.indexes.deleted)
            && (forall|p: String| result->Ok_0.live@.contains(p) == crate::catalog_state::rooted(old(log)@.indexes, p))
            && (eligible_paths(old(log)@.indexes).len() == 0 ==>
                final(log)@ == old(log)@ && final(device)@ == old(device)@)
            && (eligible_paths(old(log)@.indexes).len() > 0 ==>
                final(device)@.durable == final(log)@.offset
                && final(device)@.cursor == final(log)@.offset
                && final(log)@.floor == final(log)@.offset
                && final(log)@.history.drop_last() == old(log)@.history
                && final(log)@.history.len() == old(log)@.history.len() + 1),
))]
pub fn mark<R: Resolver, D: crate::wal_device::Writer>(
    log: &mut LogState,
    resolver: &mut R,
    run: &str,
    device: &mut D,
) -> Result<Marked, crate::catalog_append::Error<R::Error, D::Error>> {
    let live = log.indexes().roots();
    let candidates = log.indexes().candidates(&live);
    #[cfg(straw_verus)]
    proof! {
        assert forall|p: String| candidates@.contains(p) == eligible_paths(log@.indexes).contains(p) by {
            assert(candidates@.contains(p) == crate::catalog_state::eligible(log@.indexes, p));
        }
        assert(candidates@ =~= eligible_paths(log@.indexes));
    }
    if !candidates.is_empty() {
        let paths = snapshot(&candidates);
        let value = event(&paths);
        crate::catalog_append::append(log, resolver, run, value, device)?;
        #[cfg(straw_verus)]
        proof! {
            append_marks(crate::catalog_frame::Context { run: run@, namespace: resolver@ },
                old(log)@, log@, value, candidates@);
        }
    } else {
        #[cfg(straw_verus)]
        proof! {
            assert(candidates@ =~= Set::<String>::empty());
            assert(log@.indexes.deleted.union(candidates@) =~= log@.indexes.deleted);
            assert(log@.indexes.pending.union(candidates@.difference(log@.indexes.deleted)) =~= log@.indexes.pending);
            assert(marked(log@.indexes, candidates@) == log@.indexes);
        }
    }
    let targets = snapshot(&log.indexes().pending_deletions);
    Ok(Marked { live, targets })
}
