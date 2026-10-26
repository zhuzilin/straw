//! Persist exactly the completed deletion targets without reinterpreting paths.
//! The caller separately establishes durable physical absence of these names.
use crate::catalog_decode::Resolver;
use crate::catalog_log::LogState;
use serde_json::Value;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg(straw_verus)]
verus! {
    use crate::catalog_state::{State, Operation};
    use crate::json_specs::{field, json, Json};
    use crate::gc_mark::{names, string_values};
    pub open spec fn receipt_event(value: Value, paths: Set<String>) -> bool {
        json(value) is Object
            && field(value, "reclaimed"@).is_some()
            && field(value, "reclaimed"@)->Some_0 is Array
            && string_values(json(field(value, "reclaimed"@)->Some_0)->Array_0, paths)
            && field(value, "path_encoding"@).is_some()
            && json(field(value, "path_encoding"@)->Some_0) == Json::Number(Some(1))
            && forall|key: Seq<char>| key != "reclaimed"@ && key != "path_encoding"@
                ==> field(value, key).is_none()
    }
    pub open spec fn reclaimed(s: State, paths: Set<String>) -> State {
        let retired = paths.intersect(s.deleted);
        State { pending: s.pending.difference(retired), packs: s.packs.remove_keys(retired), ..s }
    }
    pub open spec fn receipt_record(context: crate::catalog_frame::Context,
        record: crate::catalog_log::Record, known: Map<String, Seq<String>>, paths: Set<String>) -> bool {
        exists|input: Value| receipt_event(input, paths)
            && crate::catalog_append::intended(context, input, known, record.payload, record.operations)
    }
    // The catalog encodes confirmed deletion receipts by permanent tombstones
    // whose retry entries have been removed, rather than a separate index.
    pub open spec fn completed(s: State) -> Set<String> {
        s.deleted.difference(s.pending)
    }
    pub proof fn mark_preserves_completed(s: State, paths: Set<String>)
        ensures completed(crate::gc_mark::marked(s, paths)) == completed(s),
    {
        assert(completed(crate::gc_mark::marked(s, paths)) =~= completed(s));
    }
    pub proof fn receipt_completes_exactly(s: State, paths: Set<String>)
        requires paths.subset_of(s.deleted),
        ensures completed(reclaimed(s, paths)) == completed(s).union(paths),
    {
        assert(completed(reclaimed(s, paths)) =~= completed(s).union(paths));
    }
    pub proof fn reclaim_one(s: State, paths: Set<String>, p: String)
        ensures crate::catalog_state::step(reclaimed(s, paths), Operation::Reclaim(p))
            == reclaimed(s, paths.insert(p)),
    {
        let previous = paths.intersect(s.deleted);
        let added = paths.insert(p).intersect(s.deleted);
        assert(added =~= if s.deleted.contains(p) { previous.insert(p) } else { previous });
        assert(s.pending.difference(added) =~= if s.deleted.contains(p) {
            s.pending.difference(previous).remove(p) } else { s.pending.difference(previous) });
        assert(s.packs.remove_keys(added) =~= if s.deleted.contains(p) {
            s.packs.remove_keys(previous).remove(p) } else { s.packs.remove_keys(previous) });
    }
    pub proof fn reclaim_fold(s: State, ops: Seq<Operation>, paths: Seq<String>)
        requires ops.len() == paths.len(),
            forall|i: int| 0 <= i < ops.len() ==> ops[i] == Operation::Reclaim(paths[i]),
        ensures crate::catalog_state::run(s, ops) == reclaimed(s, paths.to_set()),
        decreases ops.len(),
    {
        crate::catalog_state::run_unfold(s, ops);
        if ops.len() > 0 {
            reclaim_fold(s, ops.drop_last(), paths.drop_last());
            paths.drop_last().lemma_push_to_set_commute(paths.last());
            assert(paths.drop_last().push(paths.last()) == paths);
            reclaim_one(s, paths.drop_last().to_set(), paths.last());
        } else {
            assert(paths.to_set() =~= Set::<String>::empty());
            assert(reclaimed(s, paths.to_set()).pending =~= s.pending);
            assert(reclaimed(s, paths.to_set()).packs =~= s.packs);
        }
    }
    pub proof fn wire_receipts(ns: Map<String, String>, value: Value, wire: crate::catalog_json::Wire,
        ops: Seq<Operation>, paths: Set<String>, s: State)
        requires crate::environment::string_content_contract(),
            field(value, "reclaimed"@).is_some(), field(value, "reclaimed"@)->Some_0 is Array,
            string_values(json(field(value, "reclaimed"@)->Some_0)->Array_0, paths),
            field(value, "path_encoding"@).is_some(),
            json(field(value, "path_encoding"@)->Some_0) == Json::Number(Some(1)),
            field(value, "owners"@).is_none(), field(value, "packs"@).is_none(),
            field(value, "deleted"@).is_none(), field(value, "staging_edges"@).is_none(),
            field(value, "consumed_handoffs"@).is_none(),
            crate::catalog_json::wire_matches(value, wire), crate::catalog_decode::meaning(ns, wire, ops),
        ensures crate::catalog_state::run(s, ops) == reclaimed(s, paths),
    {
        use crate::catalog_decode::{actions, Action, edge_output, edge_map, edge_actions};
        let items = json(field(value, "reclaimed"@)->Some_0)->Array_0;
        assert forall|i: int| 0 <= i < items.len() implies wire.reclaimed@[i] == names(items)[i] by {
            assert(json(items[i]) == Json::Text(wire.reclaimed@[i]@));
        }
        assert(wire.reclaimed@ =~= names(items));
        assert(wire.owners@.len() == 0 && wire.packs@.len() == 0 && wire.deleted@.len() == 0
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
        assert(ops.len() == wire.reclaimed@.len() + 1);
        assert forall|i: int| 0 <= i < ops.len() - 1 implies ops.drop_last()[i] == Operation::Reclaim(wire.reclaimed@[i]) by {
            assert(actions(ops)[i] == Action::Reclaim(wire.reclaimed@[i]));
        }
        reclaim_fold(s, ops.drop_last(), wire.reclaimed@);
        crate::catalog_state::run_unfold(s, ops);
        assert(actions(ops).last() == Action::Receipts(wire.receipts@));
        assert(crate::catalog_state::run(s, ops).receipts =~= s.receipts);
    }
    pub proof fn append_receipts(context: crate::catalog_frame::Context, before: crate::catalog_log::State,
        after: crate::catalog_log::State, input: Value, paths: Set<String>)
        requires crate::environment::string_content_contract(),
            crate::catalog_log::valid(before), crate::catalog_log::ready(before),
            crate::catalog_log::valid(after), crate::catalog_log::ready(after),
            after.history.len() == before.history.len() + 1, after.history.drop_last() == before.history,
            receipt_event(input, paths), crate::catalog_append::intended(context, input, before.indexes.edges,
                after.history.last().payload, after.history.last().operations),
        ensures after.indexes == reclaimed(before.indexes, paths),
    {
        let (value, wire, keys) = choose|value: Value, wire: crate::catalog_json::Wire, keys: Map<String, Vec<String>>|
            crate::catalog_append::binds_run(input, value, context.run)
                && crate::json_specs::parses(after.history.last().payload, value)
                && crate::catalog_json::schema(value) && crate::catalog_json::wire_matches(value, wire)
                && crate::catalog_state::edge_view(keys) == before.indexes.edges
                && crate::catalog_decode::admitted(wire, keys)
                && crate::catalog_decode::resolved_wire(context.namespace, wire)
                && crate::catalog_decode::meaning(context.namespace, wire, after.history.last().operations);
        wire_receipts(context.namespace, value, wire, after.history.last().operations, paths, before.indexes);
        crate::catalog_log::complete_history(before);
        crate::catalog_log::complete_history(after);
        crate::catalog_log::history_unfold(after.history);
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures receipt_event(result, paths@.to_set()),
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
    object.insert("reclaimed".to_owned(), Value::Array(values));
    object.insert("path_encoding".to_owned(), Value::Number(1u64.into()));
    Value::Object(object)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(resolver).valid(), crate::catalog_log::valid(old(log)@), crate::catalog_log::ready(old(log)@),
        paths@.to_set().subset_of(old(log)@.indexes.deleted), old(device).valid(), !old(device)@.buffered,
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
            && final(log)@.indexes == reclaimed(old(log)@.indexes, paths@.to_set())
            && completed(final(log)@.indexes) == completed(old(log)@.indexes).union(paths@.to_set())
            && final(log)@.indexes.deleted == old(log)@.indexes.deleted
            && final(log)@.indexes.pending == old(log)@.indexes.pending.difference(paths@.to_set())
            && (paths.len() == 0 ==> final(log)@ == old(log)@ && final(device)@ == old(device)@)
            && (paths.len() > 0 ==> final(device)@.durable == final(log)@.offset
                && final(device)@.cursor == final(log)@.offset
                && final(log)@.floor == final(log)@.offset
                && final(log)@.history.drop_last() == old(log)@.history
                && final(log)@.history.len() == old(log)@.history.len() + 1
                && receipt_record(crate::catalog_frame::Context { run: run@, namespace: old(resolver)@ },
                    final(log)@.history.last(), old(log)@.indexes.edges, paths@.to_set())),
))]
pub fn record<R: Resolver, D: crate::wal_device::Writer>(
    log: &mut LogState,
    resolver: &mut R,
    run: &str,
    paths: &[String],
    device: &mut D,
) -> Result<(), crate::catalog_append::Error<R::Error, D::Error>> {
    #[cfg(straw_verus)]
    proof! { receipt_completes_exactly(log@.indexes, paths@.to_set()); }
    if paths.is_empty() {
        #[cfg(straw_verus)]
        proof! {
            assert(paths@.to_set() =~= Set::<String>::empty());
            assert(reclaimed(log@.indexes, paths@.to_set()).pending =~= log@.indexes.pending);
            assert(reclaimed(log@.indexes, paths@.to_set()).packs =~= log@.indexes.packs);
        }
        return Ok(());
    }
    let value = event(paths);
    crate::catalog_append::append(log, resolver, run, value, device)?;
    #[cfg(straw_verus)]
    proof! {
        assert(receipt_event(value, paths@.to_set()));
        assert(crate::catalog_append::intended(
            crate::catalog_frame::Context { run: run@, namespace: resolver@ }, value,
            old(log)@.indexes.edges, log@.history.last().payload, log@.history.last().operations));
        append_receipts(crate::catalog_frame::Context { run: run@, namespace: resolver@ },
            old(log)@, log@, value, paths@.to_set());
        assert(paths@.to_set().intersect(old(log)@.indexes.deleted) =~= paths@.to_set());
    }
    Ok(())
}
