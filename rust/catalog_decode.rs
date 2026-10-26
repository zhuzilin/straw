//! Construct catalog operations from decoded fields and namespace identities.
//! The resolver contract is a separate native namespace-adapter obligation.
use crate::catalog_state::Operation;
use crate::identities::Encoding;
use std::collections::{BTreeSet, HashMap};
#[cfg(straw_verus)]
use vstd::{prelude::*, std_specs::iter::IteratorSpec};

#[cfg(straw_verus)]
verus! {
    pub trait ResolverModel: View<V = Map<String, String>> { spec fn valid(&self) -> bool; }
    pub open spec fn resolves(ns: Map<String, String>, name: String, encoding: Encoding) -> bool {
        encoding == Encoding::Resolved || (encoding == Encoding::Legacy && ns.contains_key(name))
    }
    pub open spec fn identity(ns: Map<String, String>, name: String, encoding: Encoding) -> String {
        if encoding == Encoding::Resolved { name } else { ns[name] }
    }
    pub open spec fn complete(ns: Map<String, String>, names: Seq<String>, encoding: Encoding) -> bool {
        forall|i: int| 0 <= i < names.len() ==> resolves(ns, names[i], encoding)
    }
    pub open spec fn path_set(ns: Map<String, String>, names: Seq<String>, encoding: Encoding) -> Set<String> {
        Seq::new(names.len(), |i: int| identity(ns, names[i], encoding)).to_set()
    }
    pub proof fn paths_push(ns: Map<String, String>, names: Seq<String>, encoding: Encoding, name: String)
        ensures path_set(ns, names.push(name), encoding)
            == path_set(ns, names, encoding).insert(identity(ns, name, encoding)),
    {
        let before = Seq::new(names.len(), |i: int| identity(ns, names[i], encoding));
        let after = Seq::new(names.push(name).len(), |i: int| identity(ns, names.push(name)[i], encoding));
        assert(after =~= before.push(identity(ns, name, encoding)));
        assert forall|p: String| before.push(identity(ns, name, encoding)).to_set().contains(p)
            == before.to_set().insert(identity(ns, name, encoding)).contains(p) by {
            let after = before.push(identity(ns, name, encoding));
            if after.to_set().contains(p) {
                let i = choose|i: int| 0 <= i < after.len() && after[i] == p;
                if i < before.len() { assert(before[i] == p); }
            } else if before.to_set().contains(p) {
                let i = choose|i: int| 0 <= i < before.len() && before[i] == p;
                assert(after[i] == p);
            } else if p == identity(ns, name, encoding) {
                assert(after[before.len() as int] == p);
            }
        }
        assert(after.to_set() =~= before.to_set().insert(identity(ns, name, encoding)));
    }
    pub enum Action {
        Owner(String, Set<String>), Release(String), Pack(String, bool),
        Mark(String), Reclaim(String), Edges(String, Seq<String>), Receipts(Seq<String>),
    }
    pub open spec fn action(op: Operation) -> Action {
        match op {
            Operation::Owner(k, paths) => Action::Owner(k, paths@),
            Operation::Release(k) => Action::Release(k),
            Operation::Pack(p, sealed) => Action::Pack(p, sealed),
            Operation::Mark(p) => Action::Mark(p), Operation::Reclaim(p) => Action::Reclaim(p),
            Operation::Edges(k, children) => Action::Edges(k, children@),
            Operation::Receipts(ids) => Action::Receipts(ids@),
        }
    }
    pub open spec fn actions(ops: Seq<Operation>) -> Seq<Action> { Seq::new(ops.len(), |i: int| action(ops[i])) }
    pub proof fn actions_push(ops: Seq<Operation>, op: Operation)
        ensures actions(ops.push(op)) == actions(ops).push(action(op)),
    { assert(actions(ops.push(op)) =~= actions(ops).push(action(op))); }
    pub open spec fn owner_action(ns: Map<String, String>, item: (String, Option<Vec<String>>), encoding: Encoding) -> Action {
        match item.1 {
            None => Action::Release(item.0),
            Some(names) => Action::Owner(item.0, path_set(ns, names@, encoding)),
        }
    }
    pub open spec fn owner_actions(ns: Map<String, String>, items: Seq<(String, Option<Vec<String>>)>, encoding: Encoding) -> Seq<Action> {
        Seq::new(items.len(), |i: int| owner_action(ns, items[i], encoding))
    }
    pub open spec fn owners_complete(ns: Map<String, String>, items: Seq<(String, Option<Vec<String>>)>, encoding: Encoding) -> bool {
        forall|i: int| 0 <= i < items.len() ==> match (#[trigger] items[i]).1 { None => true, Some(names) => complete(ns, names@, encoding) }
    }
}
#[cfg(not(straw_verus))]
pub trait ResolverModel {}
#[cfg(not(straw_verus))]
impl<T> ResolverModel for T {}

/// Standard bulk set construction is supplied by the native collection adapter.
#[cfg_attr(straw_verus, verus_verify)]
pub trait KeySet {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::environment::string_order_contract(),
        ensures result@ == values@.to_set(),
    ))]
    fn from_values(values: Vec<String>) -> BTreeSet<String>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Resolver: ResolverModel {
    type Error;
    type Keys: KeySet;
    /// Clear derived lookup caches without changing the namespace they resolve.
    #[cfg_attr(straw_verus, verus_spec(requires old(self).valid(), ensures final(self).valid(), final(self)@ == old(self)@))]
    fn reset(&mut self) {}
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).valid(), final(self)@ == old(self)@,
            result.is_ok() ==> resolves(old(self)@, *name, encoding)
                && result->Ok_0 == identity(old(self)@, *name, encoding),
    ))]
    // Borrow the decoded String key used by the namespace model; no new allocation.
    #[allow(clippy::ptr_arg)]
    fn resolve(&mut self, name: &String, encoding: Encoding) -> Result<String, Self::Error>;
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(resolver).valid(), crate::environment::string_order_contract(),
    ensures final(resolver).valid(), final(resolver)@ == old(resolver)@,
        result.is_ok() ==> complete(old(resolver)@, names@, encoding)
            && result->Ok_0@ == path_set(old(resolver)@, names@, encoding),
))]
fn resolve_paths<R: Resolver>(
    resolver: &mut R,
    names: &[String],
    encoding: Encoding,
) -> Result<BTreeSet<String>, R::Error> {
    let mut result = Vec::with_capacity(names.len());
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= names.len(), resolver.valid(), resolver@ == old(resolver)@, crate::environment::string_order_contract(),
            complete(resolver@, names@.take(i as int), encoding),
            result@.to_set() == path_set(resolver@, names@.take(i as int), encoding),
        decreases names.len() - i,
    ))]
    while i < names.len() {
        let path = resolver.resolve(&names[i], encoding)?;
        #[cfg(straw_verus)]
        proof! {
            paths_push(resolver@, names@.take(i as int), encoding, names[i as int]);
            assert(names@.take(i + 1) == names@.take(i as int).push(names[i as int]));
        }
        #[cfg(straw_verus)]
        proof! { result@.lemma_push_to_set_commute(path); }
        result.push(path);
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! { assert(names@.take(i as int) == names@); }
    Ok(R::Keys::from_values(result))
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(resolver).valid(), crate::environment::string_order_contract(),
    ensures final(resolver).valid(), final(resolver)@ == old(resolver)@,
        result.is_ok() ==> actions(final(operations)@) == actions(old(operations)@) + owner_actions(old(resolver)@, owners@, encoding)
            && owners_complete(old(resolver)@, owners@, encoding),
))]
pub fn append_owners<R: Resolver>(
    resolver: &mut R,
    owners: crate::catalog_json::OwnerList,
    encoding: Encoding,
    operations: &mut Vec<Operation>,
) -> Result<(), R::Error> {
    let mut entries = owners.into_iter();
    #[cfg(straw_verus)]
    let mut i = verus_exec_expr!(Ghost(0nat));
    #[cfg_attr(straw_verus, verus_spec(
        invariant entries.decrease().is_some(), i@ <= owners@.len(),
            entries.remaining().len() + i@ == owners@.len(),
            entries.remaining() == owners@.subrange(i@ as int, owners@.len() as int),
            resolver.valid(), resolver@ == old(resolver)@, crate::environment::string_order_contract(),
            actions(operations@) == actions(old(operations)@) + owner_actions(resolver@, owners@.take(i@ as int), encoding),
            owners_complete(resolver@, owners@.take(i@ as int), encoding),
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        let Some((owner, names)) = entries.next() else {
            #[cfg(straw_verus)]
            proof! { assert(i@ == owners@.len()); assert(owners@.take(i@ as int) == owners@); }
            return Ok(());
        };
        let op = match names {
            None => Operation::Release(owner),
            Some(names) => Operation::Owner(owner, resolve_paths(resolver, &names, encoding)?),
        };
        #[cfg(straw_verus)]
        proof! {
            assert((owner, names) == owners@[i@ as int]);
            actions_push(operations@, op);
            assert(owner_actions(resolver@, owners@.take((i@ + 1) as int), encoding)
                =~= owner_actions(resolver@, owners@.take(i@ as int), encoding).push(action(op)));
            let next = i@ + 1;
            *i.borrow_mut() = next;
        }
        operations.push(op);
    }
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn pack_actions(ns: Map<String, String>, items: Seq<(String, bool)>, encoding: Encoding) -> Seq<Action> {
        Seq::new(items.len(), |i: int| Action::Pack(identity(ns, items[i].0, encoding), items[i].1))
    }
    pub open spec fn packs_complete(ns: Map<String, String>, items: Seq<(String, bool)>, encoding: Encoding) -> bool {
        forall|i: int| 0 <= i < items.len() ==> resolves(ns, (#[trigger] items[i]).0, encoding)
    }
    pub open spec fn deletion_actions(ns: Map<String, String>, names: Seq<String>, encoding: Encoding, reclaimed: bool) -> Seq<Action> {
        Seq::new(names.len(), |i: int| if reclaimed { Action::Reclaim(identity(ns, names[i], encoding)) }
            else { Action::Mark(identity(ns, names[i], encoding)) })
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(resolver).valid(),
    ensures final(resolver).valid(), final(resolver)@ == old(resolver)@,
        result.is_ok() ==> actions(final(operations)@) == actions(old(operations)@) + pack_actions(old(resolver)@, packs@, encoding)
            && packs_complete(old(resolver)@, packs@, encoding),
))]
fn append_packs<R: Resolver>(
    resolver: &mut R,
    packs: Vec<(String, bool)>,
    encoding: Encoding,
    operations: &mut Vec<Operation>,
) -> Result<(), R::Error> {
    let mut entries = packs.into_iter();
    #[cfg(straw_verus)]
    let mut i = verus_exec_expr!(Ghost(0nat));
    #[cfg_attr(straw_verus, verus_spec(
        invariant entries.decrease().is_some(), i@ <= packs@.len(),
            entries.remaining().len() + i@ == packs@.len(),
            entries.remaining() == packs@.subrange(i@ as int, packs@.len() as int),
            resolver.valid(), resolver@ == old(resolver)@,
            actions(operations@) == actions(old(operations)@) + pack_actions(resolver@, packs@.take(i@ as int), encoding),
            packs_complete(resolver@, packs@.take(i@ as int), encoding),
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        let Some((path, sealed)) = entries.next() else {
            #[cfg(straw_verus)]
            proof! { assert(i@ == packs@.len()); assert(packs@.take(i@ as int) == packs@); }
            return Ok(());
        };
        let key = resolver.resolve(&path, encoding)?;
        let op = Operation::Pack(key, sealed);
        #[cfg(straw_verus)]
        proof! {
            assert((path, sealed) == packs@[i@ as int]);
            actions_push(operations@, op);
            assert(pack_actions(resolver@, packs@.take((i@ + 1) as int), encoding)
                =~= pack_actions(resolver@, packs@.take(i@ as int), encoding).push(action(op)));
            let next = i@ + 1;
            *i.borrow_mut() = next;
        }
        operations.push(op);
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(resolver).valid(),
    ensures final(resolver).valid(), final(resolver)@ == old(resolver)@,
        result.is_ok() ==> actions(final(operations)@) == actions(old(operations)@) + deletion_actions(old(resolver)@, names@, encoding, reclaimed)
            && complete(old(resolver)@, names@, encoding),
))]
fn append_deletions<R: Resolver>(
    resolver: &mut R,
    names: Vec<String>,
    encoding: Encoding,
    reclaimed: bool,
    operations: &mut Vec<Operation>,
) -> Result<(), R::Error> {
    let mut entries = names.into_iter();
    #[cfg(straw_verus)]
    let mut i = verus_exec_expr!(Ghost(0nat));
    #[cfg_attr(straw_verus, verus_spec(
        invariant entries.decrease().is_some(), i@ <= names@.len(),
            entries.remaining().len() + i@ == names@.len(),
            entries.remaining() == names@.subrange(i@ as int, names@.len() as int),
            resolver.valid(), resolver@ == old(resolver)@,
            actions(operations@) == actions(old(operations)@) + deletion_actions(resolver@, names@.take(i@ as int), encoding, reclaimed),
            complete(resolver@, names@.take(i@ as int), encoding),
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        let Some(path) = entries.next() else {
            #[cfg(straw_verus)]
            proof! { assert(i@ == names@.len()); assert(names@.take(i@ as int) == names@); }
            return Ok(());
        };
        let key = resolver.resolve(&path, encoding)?;
        let op = if reclaimed {
            Operation::Reclaim(key)
        } else {
            Operation::Mark(key)
        };
        #[cfg(straw_verus)]
        proof! {
            assert(path == names@[i@ as int]);
            actions_push(operations@, op);
            assert(deletion_actions(resolver@, names@.take((i@ + 1) as int), encoding, reclaimed)
                =~= deletion_actions(resolver@, names@.take(i@ as int), encoding, reclaimed).push(action(op)));
            let next = i@ + 1;
            *i.borrow_mut() = next;
        }
        operations.push(op);
    }
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn edge_values(items: Seq<(String, Option<Vec<String>>)>) -> bool {
        forall|i: int| 0 <= i < items.len() ==> (#[trigger] items[i]).1.is_some()
    }
    pub open spec fn edge_map(items: Seq<(String, Option<Vec<String>>)>) -> Map<String, Vec<String>>
        decreases items.len(),
    {
        if items.len() == 0 { Map::empty() } else {
            match items.last().1 {
                Some(children) => edge_map(items.drop_last()).insert(items.last().0, children),
                None => edge_map(items.drop_last()),
            }
        }
    }
    pub open spec fn edge_actions(items: Seq<(String, Vec<String>)>) -> Seq<Action> {
        Seq::new(items.len(), |i: int| Action::Edges(items[i].0, items[i].1@))
    }
    pub open spec fn edge_output(ops: Seq<Operation>, graph: Map<String, Vec<String>>) -> bool {
        exists|items: Seq<(String, Vec<String>)>| items.to_set() == graph.kv_pairs()
            && items.no_duplicates() && actions(ops) == edge_actions(items)
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures result.is_ok() == edge_values(edges@),
        result.is_ok() ==> result->Ok_0@ == edge_map(edges@),
))]
fn graph(edges: crate::catalog_json::OwnerList) -> Result<HashMap<String, Vec<String>>, ()> {
    let mut result = HashMap::new();
    let mut entries = edges.into_iter();
    #[cfg(straw_verus)]
    let mut i = verus_exec_expr!(Ghost(0nat));
    #[cfg(straw_verus)]
    proof! { reveal(edge_map); }
    #[cfg_attr(straw_verus, verus_spec(
        invariant entries.decrease().is_some(), i@ <= edges@.len(),
            entries.remaining().len() + i@ == edges@.len(),
            entries.remaining() == edges@.subrange(i@ as int, edges@.len() as int),
            crate::environment::string_hash_contract(),
            result@ == edge_map(edges@.take(i@ as int)), edge_values(edges@.take(i@ as int)),
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        let Some((key, children)) = entries.next() else {
            #[cfg(straw_verus)]
            proof! { assert(i@ == edges@.len()); assert(edges@.take(i@ as int) == edges@); }
            return Ok(result);
        };
        let Some(children) = children else {
            #[cfg(straw_verus)]
            proof! { assert(edges@[i@ as int].1.is_none()); assert(!edge_values(edges@)); }
            return Err(());
        };
        #[cfg(straw_verus)]
        proof! {
            assert(edges@[i@ as int] == (key, Some(children)));
            reveal(edge_map);
            assert(edges@.take((i@ + 1) as int).drop_last() == edges@.take(i@ as int));
            let next = i@ + 1;
            *i.borrow_mut() = next;
        }
        result.insert(key, children);
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires vstd::std_specs::hash::obeys_key_model::<K>(),
    ensures crate::collection_specs::remaining(result).to_set() == map@.kv_pairs(),
        crate::collection_specs::remaining(result).no_duplicates(),
))]
pub(crate) fn owned_entries<K, V>(
    map: HashMap<K, V>,
) -> std::collections::hash_map::IntoIter<K, V> {
    #[cfg(straw_verus)]
    proof! {
        broadcast use vstd::std_specs::hash::axiom_random_state_builds_valid_hashers;
        assert(vstd::std_specs::hash::builds_valid_hashers::<std::collections::hash_map::RandomState>());
    }
    map.into_iter()
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures edge_output(result@, additions@),
))]
fn graph_operations(additions: HashMap<String, Vec<String>>) -> Vec<Operation> {
    #[cfg(straw_verus)]
    proof! {
        broadcast use vstd::std_specs::hash::axiom_random_state_builds_valid_hashers;
        assert(vstd::std_specs::hash::builds_valid_hashers::<std::collections::hash_map::RandomState>());
        assert(vstd::std_specs::hash::obeys_key_model::<String>());
    }
    let mut entries = owned_entries(additions);
    #[cfg(straw_verus)]
    proof! { assert(crate::collection_specs::remaining(entries).to_set() == additions@.kv_pairs()); }
    #[cfg(straw_verus)]
    let original = verus_exec_expr!(Ghost(crate::collection_specs::remaining(entries)));
    #[cfg(straw_verus)]
    let mut i = verus_exec_expr!(Ghost(0nat));
    let mut operations = Vec::new();
    #[cfg_attr(straw_verus, verus_spec(
        invariant i@ <= original@.len(), original@.to_set() == additions@.kv_pairs(), original@.no_duplicates(),
            crate::collection_specs::remaining(entries).len() + i@ == original@.len(),
            crate::collection_specs::remaining(entries) == original@.subrange(i@ as int, original@.len() as int),
            actions(operations@) == edge_actions(original@.take(i@ as int)),
        decreases crate::collection_specs::remaining(entries).len(),
    ))]
    loop {
        let Some((key, children)) = entries.next() else {
            #[cfg(straw_verus)]
            proof! {
                assert(i@ == original@.len()); assert(original@.take(i@ as int) =~= original@);
                assert(edge_output(operations@, additions@)) by {
                    assert(exists|items: Seq<(String, Vec<String>)>| items == original@
                        && items.to_set() == additions@.kv_pairs() && items.no_duplicates()
                        && actions(operations@) == edge_actions(items));
                }
            }
            return operations;
        };
        let op = Operation::Edges(key, children);
        #[cfg(straw_verus)]
        proof! {
            assert((key, children) == original@[i@ as int]);
            actions_push(operations@, op);
            assert(edge_actions(original@.take((i@ + 1) as int))
                =~= edge_actions(original@.take(i@ as int)).push(action(op)));
            let next = i@ + 1;
            *i.borrow_mut() = next;
        }
        operations.push(op);
    }
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Failure<E> {
    Wire(crate::catalog_json::DecodeError),
    Receipts,
    Graph,
    Unsupported,
    Path(E),
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn wire_encoding(wire: crate::catalog_json::Wire) -> Encoding {
        match wire.encoding { None => Encoding::Legacy, Some(1) => Encoding::Resolved, _ => Encoding::Unsupported }
    }
    pub open spec fn admitted(wire: crate::catalog_json::Wire, known: Map<String, Vec<String>>) -> bool {
        crate::catalog_event::receipt_policy(wire) && edge_values(wire.edges@)
            && crate::catalog_event::graph_names(edge_map(wire.edges@))
            && crate::consumption::compatible_graph(known, edge_map(wire.edges@))
            && wire_encoding(wire) != Encoding::Unsupported
    }
    pub open spec fn resolved_wire(ns: Map<String, String>, wire: crate::catalog_json::Wire) -> bool {
        owners_complete(ns, wire.owners@, wire_encoding(wire))
            && packs_complete(ns, wire.packs@, wire_encoding(wire))
            && complete(ns, wire.deleted@, wire_encoding(wire))
            && complete(ns, wire.reclaimed@, wire_encoding(wire))
    }
    pub open spec fn meaning(ns: Map<String, String>, wire: crate::catalog_json::Wire, ops: Seq<Operation>) -> bool {
        exists|prefix: Seq<Operation>| edge_output(prefix, edge_map(wire.edges@))
            && actions(ops) == actions(prefix)
                + owner_actions(ns, wire.owners@, wire_encoding(wire))
                + pack_actions(ns, wire.packs@, wire_encoding(wire))
                + deletion_actions(ns, wire.deleted@, wire_encoding(wire), false)
                + deletion_actions(ns, wire.reclaimed@, wire_encoding(wire), true)
                + seq![Action::Receipts(wire.receipts@)]
    }
    pub open spec fn construction_result<E>(ns: Map<String, String>, wire: crate::catalog_json::Wire,
        known: Map<String, Vec<String>>, result: Result<Vec<Operation>, Failure<E>>) -> bool {
        match result {
            Ok(ops) => admitted(wire, known) && resolved_wire(ns, wire) && meaning(ns, wire, ops@),
            Err(Failure::Wire(_)) => false,
            Err(Failure::Receipts) => !crate::catalog_event::receipt_policy(wire),
            Err(Failure::Graph) => crate::catalog_event::receipt_policy(wire)
                && (!edge_values(wire.edges@) || !crate::catalog_event::graph_names(edge_map(wire.edges@))
                    || !crate::consumption::compatible_graph(known, edge_map(wire.edges@))),
            Err(Failure::Unsupported) => crate::catalog_event::receipt_policy(wire) && edge_values(wire.edges@)
                && crate::catalog_event::graph_names(edge_map(wire.edges@))
                && crate::consumption::compatible_graph(known, edge_map(wire.edges@))
                && wire_encoding(wire) == Encoding::Unsupported,
            Err(Failure::Path(_)) => admitted(wire, known),
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(resolver).valid(), crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
    ensures final(resolver).valid(), final(resolver)@ == old(resolver)@,
        construction_result(old(resolver)@, wire, known@, result),
))]
fn construct<R: Resolver>(
    resolver: &mut R,
    wire: crate::catalog_json::Wire,
    known: &HashMap<String, Vec<String>>,
) -> Result<Vec<Operation>, Failure<R::Error>> {
    #[cfg(straw_verus)]
    proof! { reveal(construction_result); reveal(admitted); }
    if !crate::catalog_event::valid_receipts(&wire) {
        return Err(Failure::Receipts);
    }
    let additions = match graph(wire.edges) {
        Ok(additions) => additions,
        Err(()) => return Err(Failure::Graph),
    };
    if !crate::catalog_event::valid_staging_edges(known, &additions) {
        return Err(Failure::Graph);
    }
    let encoding = crate::identities::encoding(wire.encoding);
    #[cfg(straw_verus)]
    proof! {
        assert(encoding == wire_encoding(wire));
        assert(edge_values(wire.edges@));
        assert(crate::catalog_event::receipt_policy(wire));
        assert(crate::catalog_event::graph_names(edge_map(wire.edges@)));
        assert(crate::consumption::compatible_graph(known@, edge_map(wire.edges@)));
    }
    if matches!(encoding, Encoding::Unsupported) {
        return Err(Failure::Unsupported);
    }
    let mut operations = graph_operations(additions);
    #[cfg(straw_verus)]
    let prefix = verus_exec_expr!(Ghost(operations@));
    if let Err(error) = append_owners(resolver, wire.owners, encoding, &mut operations) {
        return Err(Failure::Path(error));
    }
    if let Err(error) = append_packs(resolver, wire.packs, encoding, &mut operations) {
        return Err(Failure::Path(error));
    }
    if let Err(error) = append_deletions(resolver, wire.deleted, encoding, false, &mut operations) {
        return Err(Failure::Path(error));
    }
    if let Err(error) = append_deletions(resolver, wire.reclaimed, encoding, true, &mut operations)
    {
        return Err(Failure::Path(error));
    }
    let receipts = Operation::Receipts(wire.receipts);
    #[cfg(straw_verus)]
    proof! { actions_push(operations@, receipts); }
    operations.push(receipts);
    #[cfg(straw_verus)]
    proof! {
        assert(meaning(resolver@, wire, operations@)) by {
            assert(exists|p: Seq<Operation>| p == prefix@ && edge_output(p, edge_map(wire.edges@))
                && actions(operations@) == actions(p)
                    + owner_actions(resolver@, wire.owners@, wire_encoding(wire))
                    + pack_actions(resolver@, wire.packs@, wire_encoding(wire))
                    + deletion_actions(resolver@, wire.deleted@, wire_encoding(wire), false)
                    + deletion_actions(resolver@, wire.reclaimed@, wire_encoding(wire), true)
                    + seq![Action::Receipts(wire.receipts@)]);
        }
    }
    Ok(operations)
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn decoded<E>(ns: Map<String, String>, value: serde_json::Value, known: Map<String, Vec<String>>,
        result: Result<Vec<Operation>, Failure<E>>) -> bool {
        match result {
            Err(Failure::Wire(_)) => !crate::catalog_json::schema(value),
            _ => crate::catalog_json::schema(value) && exists|wire: crate::catalog_json::Wire|
                crate::catalog_json::wire_matches(value, wire) && construction_result(ns, wire, known, result),
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(resolver).valid(), crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
    ensures final(resolver).valid(), final(resolver)@ == old(resolver)@, decoded(old(resolver)@, *value, known@, result),
))]
pub fn decode<R: Resolver>(
    resolver: &mut R,
    value: &serde_json::Value,
    known: &HashMap<String, Vec<String>>,
) -> Result<Vec<Operation>, Failure<R::Error>> {
    let wire = match crate::catalog_json::decode(value) {
        Ok(wire) => wire,
        Err(error) => return Err(Failure::Wire(error)),
    };
    let result = construct(resolver, wire, known);
    #[cfg(straw_verus)]
    proof! {
        assert(exists|w: crate::catalog_json::Wire| w == wire && crate::catalog_json::wire_matches(*value, w)
            && construction_result(old(resolver)@, w, known@, result));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_state::Indexes;
    use serde_json::json;

    #[derive(Default)]
    struct Namespace {
        calls: Vec<String>,
    }
    impl Resolver for Namespace {
        type Error = &'static str;
        type Keys = crate::gc::NativeKeySet;

        fn resolve(&mut self, name: &String, encoding: Encoding) -> Result<String, Self::Error> {
            self.calls.push(name.clone());
            match encoding {
                Encoding::Resolved => Ok(name.clone()),
                Encoding::Legacy if name == "alias" || name == "data/pack" => {
                    Ok("data/pack".into())
                }
                _ => Err("unresolved"),
            }
        }
    }

    #[test]
    fn catalog_fields_rebuild_canonical_indexes_in_order() {
        let parent = format!("staged:{}", "a".repeat(64));
        let child = format!("staged:{}", "b".repeat(64));
        let event = json!({
            "owners": {"reader": ["alias", "data/pack"], "released": null},
            "packs": {"alias": true}, "deleted": ["alias"], "reclaimed": ["data/pack"],
            "staging_edges": {&parent: [&child], &child: []}
        });
        let mut namespace = Namespace::default();
        let operations = decode(&mut namespace, &event, &HashMap::new()).unwrap();
        let mut indexes = Indexes::new();
        indexes
            .owners
            .insert("released".into(), BTreeSet::from(["old/pack".into()]));
        indexes.apply(&operations).unwrap();
        assert_eq!(
            indexes.owners,
            HashMap::from([("reader".into(), BTreeSet::from(["data/pack".into()]))])
        );
        assert!(indexes.packs.is_empty());
        assert!(indexes.pending_deletions.is_empty());
        assert_eq!(
            indexes.deleted,
            std::collections::HashSet::from(["data/pack".into()])
        );
        assert_eq!(
            indexes.staging_edges,
            HashMap::from([(parent, vec![child.clone()]), (child, vec![])])
        );

        let receipt = "c".repeat(64);
        let release = json!({"owners": {"reader": null}, "consumed_handoffs": [&receipt]});
        let operations = decode(&mut namespace, &release, &indexes.staging_edges).unwrap();
        indexes.apply(&operations).unwrap();
        assert!(indexes.owners.is_empty());
        assert!(indexes.consumed_handoffs.contains(&receipt));
    }

    #[test]
    fn failed_decoding_returns_no_partial_operations_or_graph_updates() {
        let known = HashMap::from([(format!("staged:{}", "a".repeat(64)), Vec::<String>::new())]);
        let mut namespace = Namespace::default();
        let event = json!({"owners": {"reader": ["alias", "missing"]}});
        assert!(matches!(
            decode(&mut namespace, &event, &known),
            Err(Failure::Path("unresolved"))
        ));
        assert_eq!(namespace.calls, ["alias", "missing"]);
        assert_eq!(known.len(), 1);

        namespace.calls.clear();
        let event = json!({"owners": {"reader": ["alias"]}, "consumed_handoffs": ["c".repeat(64)]});
        assert!(matches!(
            decode(&mut namespace, &event, &known),
            Err(Failure::Receipts)
        ));
        assert!(namespace.calls.is_empty());

        let event = json!({"path_encoding": 1, "owners": {"reader": ["alias"]}});
        let operations = decode(&mut namespace, &event, &known).unwrap();
        let mut indexes = Indexes::new();
        indexes.apply(&operations).unwrap();
        assert_eq!(indexes.owners["reader"], BTreeSet::from(["alias".into()]));
    }
}
