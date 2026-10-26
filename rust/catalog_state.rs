//! Persistent catalog indexes and their ordered replay transitions.
//! Wire decoding and path resolution produce operations; this module applies
//! those operations to the same indexes used by ownership and collection.
use std::collections::{BTreeSet, HashMap, HashSet};
#[cfg(straw_verus)]
use vstd::prelude::*;
#[cfg(straw_verus)]
use vstd::std_specs::iter::IteratorSpec;

#[cfg_attr(straw_verus, verus_verify)]
pub struct Indexes {
    pub owners: HashMap<String, BTreeSet<String>>,
    pub packs: HashMap<String, bool>,
    pub deleted: HashSet<String>,
    pub pending_deletions: HashSet<String>,
    pub staging_edges: HashMap<String, Vec<String>>,
    pub consumed_handoffs: HashSet<String>,
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum Operation {
    Owner(String, BTreeSet<String>),
    Release(String),
    Pack(String, bool),
    Mark(String),
    Reclaim(String),
    Edges(String, Vec<String>),
    Receipts(Vec<String>),
}

#[cfg(straw_verus)]
verus! {
    pub struct State {
        pub owners: Map<String, Set<String>>,
        pub packs: Map<String, bool>,
        pub deleted: Set<String>,
        pub pending: Set<String>,
        pub edges: Map<String, Seq<String>>,
        pub receipts: Set<String>,
    }

    pub open spec fn owner_view(m: Map<String, BTreeSet<String>>) -> Map<String, Set<String>> {
        Map::new(m.dom(), |k: String| m[k]@)
    }
    pub open spec fn edge_view(m: Map<String, Vec<String>>) -> Map<String, Seq<String>> {
        Map::new(m.dom(), |k: String| m[k]@)
    }
    impl View for Indexes {
        type V = State;
        open spec fn view(&self) -> State {
            State { owners: owner_view(self.owners@), packs: self.packs@,
                deleted: self.deleted@, pending: self.pending_deletions@,
                edges: edge_view(self.staging_edges@), receipts: self.consumed_handoffs@ }
        }
    }
    pub open spec fn valid(s: State) -> bool { s.pending.subset_of(s.deleted) }
    pub open spec fn reachable(sets: Set<BTreeSet<String>>, p: String) -> bool {
        exists|s: BTreeSet<String>| sets.contains(s) && s@.contains(p)
    }
    pub open spec fn rooted(s: State, p: String) -> bool {
        exists|owner: String| s.owners.contains_key(owner) && s.owners[owner].contains(p)
    }
    pub open spec fn eligible(s: State, p: String) -> bool {
        s.packs.contains_key(p) && s.packs[p] && !rooted(s, p) && !s.deleted.contains(p)
    }
    pub proof fn pop_pairs<K, V>(items: Seq<(&K, &V)>)
        requires items.len() > 0,
        ensures items.unref().to_set() == items.drop_first().unref().to_set().insert((*items[0].0, *items[0].1)),
    {
        assert forall|v: (K, V)| items.unref().to_set().contains(v)
            == items.drop_first().unref().to_set().insert((*items[0].0, *items[0].1)).contains(v) by {
            if items.unref().to_set().contains(v) {
                let i = choose|i: int| 0 <= i < items.len() && items.unref()[i] == v;
                if i > 0 {
                    assert(items.drop_first().unref()[i - 1] == v);
                    assert(items.drop_first().unref().to_set().contains(v));
                } else { assert(items.unref()[0] == v); }
            } else if items.drop_first().unref().to_set().contains(v) {
                let i = choose|i: int| 0 <= i < items.drop_first().len() && items.drop_first().unref()[i] == v;
                assert(items.unref()[i + 1] == v);
                assert(items.unref().to_set().contains(v));
            } else {
                assert(items.unref()[0] == (*items[0].0, *items[0].1));
            }
        }
        assert(items.unref().to_set() =~= items.drop_first().unref().to_set().insert((*items[0].0, *items[0].1)));
    }
    pub proof fn pop_set<A>(items: Seq<&A>)
        requires items.len() > 0,
        ensures items.unref().to_set() == items.drop_first().unref().to_set().insert(*items[0]),
    {
        assert forall|v: A| items.unref().to_set().contains(v)
            == items.drop_first().unref().to_set().insert(*items[0]).contains(v) by {
            if items.unref().to_set().contains(v) {
                let i = choose|i: int| 0 <= i < items.len() && items.unref()[i] == v;
                if i > 0 {
                    assert(items.drop_first().unref()[i - 1] == v);
                    assert(items.drop_first().unref().to_set().contains(v));
                } else { assert(items.unref()[0] == v); }
            } else if items.drop_first().unref().to_set().contains(v) {
                let i = choose|i: int| 0 <= i < items.drop_first().len() && items.drop_first().unref()[i] == v;
                assert(items.unref()[i + 1] == v);
                assert(items.unref().to_set().contains(v));
            } else {
                assert(items.unref()[0] == *items[0]);
            }
        }
        assert(items.unref().to_set() =~= items.drop_first().unref().to_set().insert(*items[0]));
    }
    pub proof fn reachable_insert(sets: Set<BTreeSet<String>>, added: BTreeSet<String>)
        ensures forall|p: String| reachable(sets.insert(added), p) == (reachable(sets, p) || added@.contains(p)),
    {
        assert forall|p: String| reachable(sets.insert(added), p) == (reachable(sets, p) || added@.contains(p)) by {
            if reachable(sets.insert(added), p) {
                let paths = choose|paths: BTreeSet<String>| sets.insert(added).contains(paths) && paths@.contains(p);
                if paths != added { assert(sets.contains(paths)); }
            } else if reachable(sets, p) {
                let paths = choose|paths: BTreeSet<String>| sets.contains(paths) && paths@.contains(p);
                assert(sets.insert(added).contains(paths));
            } else if added@.contains(p) {
                assert(sets.insert(added).contains(added));
            }
        }
    }
    pub proof fn roots_correspond(s: Indexes)
        ensures forall|p: String| reachable(s.owners@.values(), p) == rooted(s@, p),
    {
        assert forall|p: String| reachable(s.owners@.values(), p) == rooted(s@, p) by {
            if reachable(s.owners@.values(), p) {
                let paths = choose|paths: BTreeSet<String>| s.owners@.values().contains(paths) && paths@.contains(p);
                let key = choose|key: String| s.owners@.contains_key(key) && s.owners@[key] == paths;
                assert(s@.owners[key].contains(p));
            } else if rooted(s@, p) {
                let key = choose|key: String| s@.owners.contains_key(key) && s@.owners[key].contains(p);
                assert(s.owners@.values().contains(s.owners@[key]));
            }
        }
    }
    pub open spec fn permitted(s: State, op: Operation) -> bool {
        match op { Operation::Reclaim(p) => s.deleted.contains(p), _ => true }
    }
    pub open spec fn step(s: State, op: Operation) -> State {
        match op {
            Operation::Owner(k, paths) => State { owners: s.owners.insert(k, paths@), ..s },
            Operation::Release(k) => State { owners: s.owners.remove(k), ..s },
            Operation::Pack(p, sealed) => State { packs: s.packs.insert(p, sealed), ..s },
            Operation::Mark(p) => State { deleted: s.deleted.insert(p),
                pending: if s.deleted.contains(p) { s.pending } else { s.pending.insert(p) }, ..s },
            Operation::Reclaim(p) => if s.deleted.contains(p) {
                State { pending: s.pending.remove(p), packs: s.packs.remove(p), ..s }
            } else { s },
            Operation::Edges(k, children) => State { edges: s.edges.insert(k, children@), ..s },
            Operation::Receipts(ids) => State { receipts: s.receipts.union(ids@.to_set()), ..s },
        }
    }
    pub closed spec fn run(s: State, ops: Seq<Operation>) -> State
        decreases ops.len(),
    {
        if ops.len() == 0 { s } else { step(run(s, ops.drop_last()), ops.last()) }
    }
    pub closed spec fn accepted(s: State, ops: Seq<Operation>) -> bool
        decreases ops.len(),
    {
        ops.len() == 0 || (accepted(s, ops.drop_last())
            && permitted(run(s, ops.drop_last()), ops.last()))
    }
    pub proof fn run_empty(s: State)
        ensures run(s, Seq::empty()) == s,
    { reveal(run); }

    pub proof fn run_unfold(s: State, ops: Seq<Operation>)
        ensures run(s, ops) == if ops.len() == 0 { s }
            else { step(run(s, ops.drop_last()), ops.last()) },
    { reveal(run); }

    pub proof fn accepted_prefix(s: State, ops: Seq<Operation>, count: nat)
        requires count <= ops.len(), accepted(s, ops),
        ensures accepted(s, ops.take(count as int)),
        decreases ops.len(),
    {
        reveal(accepted);
        if count < ops.len() {
            accepted_prefix(s, ops.drop_last(), count);
            assert(ops.drop_last().take(count as int) == ops.take(count as int));
        } else { assert(ops.take(count as int) == ops); }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl Indexes {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::environment::string_hash_contract(),
        ensures valid(result@), result@.owners == Map::<String, Set<String>>::empty(),
            result@.packs == Map::<String, bool>::empty(), result@.deleted == Set::<String>::empty(),
            result@.pending == Set::<String>::empty(), result@.edges == Map::<String, Seq<String>>::empty(),
            result@.receipts == Set::<String>::empty(),
    ))]
    pub fn new() -> Self {
        Self {
            owners: HashMap::new(),
            packs: HashMap::new(),
            deleted: HashSet::new(),
            pending_deletions: HashSet::new(),
            staging_edges: HashMap::new(),
            consumed_handoffs: HashSet::new(),
        }
    }

    /// Check the whole ordered update without modifying the live indexes.
    /// Only same-event marks need scratch space; historical indexes are shared.
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::environment::string_hash_contract(),
        ensures result.is_ok() == accepted(self@, ops@),
            result.is_err() ==> result->Err_0 < ops.len()
                && accepted(self@, ops@.take(result->Err_0 as int))
                && !permitted(run(self@, ops@.take(result->Err_0 as int)), ops[result->Err_0 as int]),
    ))]
    pub fn validate(&self, ops: &[Operation]) -> Result<(), usize> {
        let mut marked: HashSet<String> = HashSet::new();
        let mut i = 0usize;
        #[cfg(straw_verus)]
        proof! { reveal(run); reveal(accepted); }
        #[cfg_attr(straw_verus, verus_spec(
            invariant i <= ops.len(), crate::environment::string_hash_contract(),
                run(self@, ops@.take(i as int)).deleted == self@.deleted.union(marked@),
                accepted(self@, ops@.take(i as int)),
            decreases ops.len() - i,
        ))]
        while i < ops.len() {
            #[cfg(straw_verus)]
            proof! {
                reveal(run); reveal(accepted);
                assert(ops@.take(i + 1).drop_last() == ops@.take(i as int));
            }
            match &ops[i] {
                Operation::Mark(path) => {
                    marked.insert(path.clone());
                }
                Operation::Reclaim(path) => {
                    if !self.deleted.contains(path) && !marked.contains(path) {
                        #[cfg(straw_verus)]
                        proof! {
                            if accepted(self@, ops@) {
                                accepted_prefix(self@, ops@, (i + 1) as nat);
                                assert(permitted(run(self@, ops@.take(i as int)), ops[i as int]));
                            }
                        }
                        return Err(i);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        #[cfg(straw_verus)]
        proof! { assert(ops@.take(i as int) == ops@); }
        Ok(())
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
        ensures forall|p: String| result@.contains(p) == rooted(self@, p),
    ))]
    pub fn roots(&self) -> HashSet<String> {
        let mut result = HashSet::new();
        let mut entries = self.owners.values();
        #[cfg_attr(straw_verus, verus_spec(
            invariant entries.decrease().is_some(), crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
                forall|p: String| #![trigger reachable(self.owners@.values(), p)] (result@.contains(p) || reachable(entries.remaining().unref().to_set(), p)) == reachable(self.owners@.values(), p),
            decreases entries.decrease().unwrap(),
        ))]
        loop {
            #[cfg(straw_verus)]
            proof! {
                if entries.remaining().len() > 0 {
                    pop_set(entries.remaining());
                    reachable_insert(entries.remaining().drop_first().unref().to_set(), *entries.remaining()[0]);
                    assert forall|p: String| (result@.contains(p) || entries.remaining()[0]@.contains(p)
                        || reachable(entries.remaining().drop_first().unref().to_set(), p))
                        == reachable(self.owners@.values(), p) by {
                        assert(reachable(entries.remaining().unref().to_set(), p)
                            == (reachable(entries.remaining().drop_first().unref().to_set(), p) || (*entries.remaining()[0])@.contains(p)));
                        assert((result@.contains(p) || reachable(entries.remaining().unref().to_set(), p)) == reachable(self.owners@.values(), p));
                    };
                }
            }
            let Some(paths) = entries.next() else {
                #[cfg(straw_verus)]
                proof! {
                    roots_correspond(*self);
                    assert(entries.remaining().unref().to_set() =~= Set::<BTreeSet<String>>::empty());
                    assert forall|p: String| result@.contains(p) == rooted(self@, p) by {};
                }
                return result;
            };
            #[cfg(straw_verus)]
            proof! {
                assert forall|p: String| (result@.contains(p) || paths@.contains(p)
                    || reachable(entries.remaining().unref().to_set(), p)) == reachable(self.owners@.values(), p) by {};
            }
            extend_roots(&mut result, paths);
            #[cfg(straw_verus)]
            proof! {
                assert forall|p: String| #![trigger reachable(self.owners@.values(), p)] (result@.contains(p) || reachable(entries.remaining().unref().to_set(), p)) == reachable(self.owners@.values(), p) by {};
            }
        }
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::environment::string_hash_contract(),
            forall|p: String| live@.contains(p) == rooted(self@, p),
        ensures forall|p: String| result@.contains(p) == eligible(self@, p),
    ))]
    pub fn candidates(&self, live: &HashSet<String>) -> HashSet<String> {
        let mut result = HashSet::new();
        let mut entries = self.packs.iter();
        #[cfg_attr(straw_verus, verus_spec(
            invariant entries.decrease().is_some(), crate::environment::string_hash_contract(),
                forall|p: String| live@.contains(p) == rooted(self@, p),
                entries.remaining().unref().to_set().subset_of(self.packs@.kv_pairs()),
                forall|p: String| (result@.contains(p) || (eligible(self@, p)
                    && entries.remaining().unref().to_set().contains((p, true)))) == eligible(self@, p),
            decreases entries.decrease().unwrap(),
        ))]
        loop {
            #[cfg(straw_verus)]
            proof! {
                if entries.remaining().len() > 0 {
                    pop_pairs(entries.remaining());
                    let path = *entries.remaining()[0].0;
                    let sealed = *entries.remaining()[0].1;
                    assert(entries.remaining().unref()[0] == (path, sealed));
                    assert(entries.remaining().unref().to_set().contains((path, sealed)));
                    assert(self.packs@.kv_pairs().contains((path, sealed)));
                    assert(self.packs@.contains_key(path));
                    assert(self.packs@[path] == sealed);
                }
            }
            let Some((path, sealed)) = entries.next() else {
                return result;
            };
            #[cfg(straw_verus)]
            proof! {
                assert(self.packs@.contains_key(*path));
                assert(self.packs@[*path] == *sealed);
                assert forall|p: String| (result@.contains(p) || (p == *path && eligible(self@, p))
                    || (eligible(self@, p) && entries.remaining().unref().to_set().contains((p, true))))
                    == eligible(self@, p) by {};
            }
            let status = crate::protocol::PackStatus {
                sealed: *sealed,
                owned: live.contains(path),
                tombstoned: self.deleted.contains(path),
            };
            if crate::protocol::candidate(status) {
                result.insert(path.clone());
            }
            #[cfg(straw_verus)]
            proof! {
                assert forall|p: String| (result@.contains(p) || (eligible(self@, p)
                    && entries.remaining().unref().to_set().contains((p, true)))) == eligible(self@, p) by {};
            }
        }
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
            valid(old(self)@),
        ensures result == permitted(old(self)@, *op),
            final(self)@ == step(old(self)@, *op), valid(final(self)@),
            !result ==> final(self)@ == old(self)@,
    ))]
    pub fn apply_one(&mut self, op: &Operation) -> bool {
        match op {
            Operation::Owner(key, paths) => {
                self.owners.insert(key.clone(), paths.clone());
            }
            Operation::Release(key) => {
                self.owners.remove(key);
            }
            Operation::Pack(path, sealed) => {
                self.packs.insert(path.clone(), *sealed);
            }
            Operation::Mark(path) => {
                crate::reclamation::mark(
                    &mut self.deleted,
                    &mut self.pending_deletions,
                    path.clone(),
                );
            }
            Operation::Reclaim(path) => {
                if !crate::reclamation::retire(&self.deleted, &mut self.pending_deletions, path) {
                    return false;
                }
                self.packs.remove(path);
            }
            Operation::Edges(key, children) => {
                self.staging_edges.insert(key.clone(), children.clone());
            }
            Operation::Receipts(ids) => {
                crate::consumption::record_receipts(&mut self.consumed_handoffs, ids);
            }
        }
        #[cfg(straw_verus)]
        proof! {
            assert(self@.owners =~= step(old(self)@, *op).owners);
            assert(self@.edges =~= step(old(self)@, *op).edges);
        }
        true
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::environment::string_hash_contract(), crate::environment::string_order_contract(), valid(old(self)@),
        ensures valid(final(self)@),
            result.is_ok() == accepted(old(self)@, ops@),
            result.is_ok() ==> final(self)@ == run(old(self)@, ops@) && accepted(old(self)@, ops@),
            result.is_err() ==> result->Err_0 < ops.len()
                && final(self)@ == run(old(self)@, ops@.take(result->Err_0 as int))
                && accepted(old(self)@, ops@.take(result->Err_0 as int))
                && !permitted(final(self)@, ops[result->Err_0 as int]),
    ))]
    pub fn apply(&mut self, ops: &[Operation]) -> Result<(), usize> {
        let mut i = 0usize;
        #[cfg(straw_verus)]
        proof! { reveal(run); reveal(accepted); }
        #[cfg_attr(straw_verus, verus_spec(
            invariant i <= ops.len(), valid(self@), crate::environment::string_hash_contract(),
                crate::environment::string_order_contract(), self@ == run(old(self)@, ops@.take(i as int)),
                accepted(old(self)@, ops@.take(i as int)),
            decreases ops.len() - i,
        ))]
        while i < ops.len() {
            #[cfg(straw_verus)]
            proof! {
                reveal(run); reveal(accepted);
                assert(ops@.take(i + 1).drop_last() == ops@.take(i as int));
            }
            if !self.apply_one(&ops[i]) {
                #[cfg(straw_verus)]
                proof! {
                    if accepted(old(self)@, ops@) {
                        accepted_prefix(old(self)@, ops@, (i + 1) as nat);
                        assert(permitted(run(old(self)@, ops@.take(i as int)), ops[i as int]));
                    }
                }
                return Err(i);
            }
            i += 1;
        }
        #[cfg(straw_verus)]
        proof! { assert(ops@.take(i as int) == ops@); }
        Ok(())
    }
}

#[cfg_attr(straw_verus, verus_spec(
    requires crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
    ensures final(result)@ == old(result)@.union(paths@),
))]
fn extend_roots(result: &mut HashSet<String>, paths: &BTreeSet<String>) {
    let mut entries = paths.iter();
    #[cfg_attr(straw_verus, verus_spec(
        invariant entries.decrease().is_some(), crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
            result@.union(entries.remaining().unref().to_set()) == old(result)@.union(paths@),
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        #[cfg(straw_verus)]
        proof! { if entries.remaining().len() > 0 { pop_set(entries.remaining()); } }
        let Some(path) = entries.next() else {
            return;
        };
        result.insert(path.clone());
        #[cfg(straw_verus)]
        proof! {
            assert(result@.union(entries.remaining().unref().to_set()) =~= old(result)@.union(paths@));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_uses_only_preceding_marks_and_leaves_indexes_unchanged() {
        let mut indexes = Indexes::new();
        let path = "raw/pack".to_string();
        let owner = Operation::Owner("reader".into(), BTreeSet::from([path.clone()]));
        let invalid = [
            owner,
            Operation::Reclaim(path.clone()),
            Operation::Mark(path.clone()),
        ];
        assert_eq!(indexes.validate(&invalid), Err(1));
        assert!(indexes.owners.is_empty());
        assert!(indexes.deleted.is_empty());
        assert!(indexes.pending_deletions.is_empty());
        let valid = [
            Operation::Mark(path.clone()),
            Operation::Reclaim(path.clone()),
        ];
        assert_eq!(indexes.validate(&valid), Ok(()));
        assert!(indexes.deleted.is_empty());
        assert_eq!(indexes.apply(&valid), Ok(()));
        assert!(indexes.deleted.contains(&path));
        assert!(indexes.pending_deletions.is_empty());
        // Durable old marks remain valid even when no mark occurs in this event.
        assert_eq!(indexes.validate(&[Operation::Reclaim(path)]), Ok(()));
    }
}
