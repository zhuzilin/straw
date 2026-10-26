//! Complete pack reachability, including cached subgraphs and cyclic edges.
//! The source reads one immutable node; the traversal and cache are verified here.
use std::collections::{BTreeSet, HashMap, HashSet};
#[cfg(straw_verus)]
use vstd::{prelude::*, std_specs::iter::IteratorSpec};

#[cfg_attr(straw_verus, verus_verify)]
pub struct Expansion<T> {
    pub pack: String,
    pub children: Vec<(String, T)>,
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum Failure<E> {
    Read(E),
    Reclaimed,
}

#[cfg(straw_verus)]
verus! {
    pub struct Node {
        pub pack: String,
        pub children: Set<String>,
    }
    #[verifier::reject_recursive_types(T)]
    pub struct Graph<T> {
        pub nodes: Map<String, Node>,
        pub key: spec_fn(T) -> String,
    }
    pub trait SourceModel: View<V = Graph<Self::Reference>> {
        type Reference;
    }
    pub open spec fn valid<T>(g: Graph<T>) -> bool {
        forall|k: String| g.nodes.contains_key(k) ==> g.nodes[k].children.subset_of(g.nodes.dom())
    }
    pub open spec fn keys<T>(items: Seq<(String, T)>) -> Set<String> {
        items.map(|i: int, pair: (String, T)| pair.0).to_set()
    }
    pub open spec fn bound<T>(g: Graph<T>, items: Seq<(String, T)>) -> bool {
        forall|i: int| 0 <= i < items.len() ==> g.nodes.contains_key(items[i].0) && (g.key)(items[i].1) == items[i].0
    }
    #[verifier::opaque]
    pub open spec fn path<T>(g: Graph<T>, p: Seq<String>) -> bool {
        p.len() > 0 && (forall|i: int| 0 <= i < p.len() ==> g.nodes.contains_key(p[i]))
            && (forall|i: int| 0 <= i < p.len() - 1 ==> g.nodes[p[i]].children.contains(p[i + 1]))
    }
    pub open spec fn reaches<T>(g: Graph<T>, root: String, end: String) -> bool {
        exists|p: Seq<String>| path(g, p) && p[0] == root && p.last() == end
    }
    pub open spec fn nodes<T>(g: Graph<T>, root: String) -> Set<String> {
        g.nodes.dom().filter(|k: String| reaches(g, root, k))
    }
    pub open spec fn packs<T>(g: Graph<T>, root: String) -> Set<String> {
        nodes(g, root).map(|k: String| g.nodes[k].pack)
    }
    pub open spec fn cached<T>(g: Graph<T>, cache: Map<String, BTreeSet<String>>) -> bool {
        forall|k: String| cache.contains_key(k) ==> g.nodes.contains_key(k) && cache[k]@ == packs(g, k)
    }
    pub proof fn empty_cache<T>(g: Graph<T>)
        ensures cached(g, Map::<String, BTreeSet<String>>::empty()),
    {}
    pub proof fn witness<T>(g: Graph<T>, p: Seq<String>)
        requires path(g, p), p.len() > 0,
        ensures reaches(g, p[0], p.last()),
    {
        assert(exists|q: Seq<String>| #[trigger] path(g, q) && q[0] == p[0] && q.last() == p.last()) by {
            assert(path(g, p));
        }
    }
    pub proof fn root_reaches<T>(g: Graph<T>, root: String)
        requires g.nodes.contains_key(root),
        ensures nodes(g, root).contains(root), packs(g, root).contains(g.nodes[root].pack),
    {
        reveal(path);
        assert(path(g, seq![root]));
        witness(g, seq![root]);
        assert(reaches(g, root, root));
        assert(nodes(g, root).contains(root));
        let pack_of = |name: String| g.nodes[name].pack;
        assert(pack_of(root) == g.nodes[root].pack);
        assert(nodes(g, root).map(pack_of).contains(pack_of(root)));
    }
    pub proof fn append_edge<T>(g: Graph<T>, root: String, a: String, b: String)
        requires valid(g), reaches(g, root, a), g.nodes[a].children.contains(b),
        ensures reaches(g, root, b),
    {
        reveal(path);
        let p = choose|p: Seq<String>| path(g, p) && p[0] == root && p.last() == a;
        assert(path(g, p.push(b)));
        witness(g, p.push(b));
        assert(reaches(g, root, b));
    }
    pub proof fn transitive<T>(g: Graph<T>, root: String, middle: String, end: String)
        requires reaches(g, root, middle), reaches(g, middle, end),
        ensures reaches(g, root, end),
    {
        reveal(path);
        let first = choose|p: Seq<String>| path(g, p) && p[0] == root && p.last() == middle;
        let last = choose|p: Seq<String>| path(g, p) && p[0] == middle && p.last() == end;
        let joined = first + last.drop_first();
        assert forall|i: int| 0 <= i < joined.len() - 1 implies g.nodes[joined[i]].children.contains(joined[i + 1]) by {
            if i < first.len() - 1 { assert(joined[i] == first[i]); }
            else { assert(joined[i] == last[i - first.len() + 1]); }
        }
        assert(path(g, joined));
        witness(g, joined);
        assert(reaches(g, root, end));
    }
    pub proof fn subclosure<T>(g: Graph<T>, root: String, child: String)
        requires nodes(g, root).contains(child),
        ensures packs(g, child).subset_of(packs(g, root)),
    {
        assert forall|p: String| packs(g, child).contains(p) implies packs(g, root).contains(p) by {
            let end = choose|end: String| nodes(g, child).contains(end) && g.nodes[end].pack == p;
            transitive(g, root, child, end);
            assert(nodes(g, root).contains(end));
        }
    }
    pub open spec fn covered<T>(g: Graph<T>, visited: Set<String>, seen: Set<String>, result: Set<String>) -> bool {
        forall|k: String| visited.contains(k) ==> packs(g, k).subset_of(result)
            || (result.contains(g.nodes[k].pack) && g.nodes[k].children.subset_of(seen))
    }
    pub proof fn coverage_grows<T>(g: Graph<T>, visited: Set<String>, seen: Set<String>, result: Set<String>,
        more_seen: Set<String>, more_result: Set<String>)
        requires covered(g, visited, seen, result), seen.subset_of(more_seen), result.subset_of(more_result),
        ensures covered(g, visited, more_seen, more_result),
    {}
    pub proof fn path_covered<T>(g: Graph<T>, p: Seq<String>, seen: Set<String>, result: Set<String>)
        requires path(g, p), seen.contains(p[0]), covered(g, seen, seen, result),
        ensures result.contains(g.nodes[p.last()].pack),
        decreases p.len(),
    {
        reveal(path);
        if packs(g, p[0]).subset_of(result) {
            assert(reaches(g, p[0], p.last()));
            assert(nodes(g, p[0]).contains(p.last()));
            assert(packs(g, p[0]).contains(g.nodes[p.last()].pack));
        } else if p.len() > 1 {
            assert(path(g, p.drop_first()));
            path_covered(g, p.drop_first(), seen, result);
        }
    }
    pub proof fn complete<T>(g: Graph<T>, root: String, seen: Set<String>, result: Set<String>)
        requires seen.contains(root), covered(g, seen, seen, result), result.subset_of(packs(g, root)),
        ensures result == packs(g, root),
    {
        assert forall|pack: String| packs(g, root).contains(pack) implies result.contains(pack) by {
            let end = choose|end: String| nodes(g, root).contains(end) && g.nodes[end].pack == pack;
            let p = choose|p: Seq<String>| path(g, p) && p[0] == root && p.last() == end;
            path_covered(g, p, seen, result);
        }
        assert(result =~= packs(g, root));
    }
    pub proof fn keys_push<T>(items: Seq<(String, T)>, key: String, value: T)
        ensures keys(items.push((key, value))) == keys(items).insert(key),
    {
        let before = items.map(|i: int, p: (String, T)| p.0);
        assert(items.push((key, value)).map(|i: int, p: (String, T)| p.0) =~= before.push(key));
        before.lemma_push_to_set_commute(key);
    }
    pub proof fn key_membership<T>(items: Seq<(String, T)>, key: String)
        ensures keys(items).contains(key) == (exists|i: int| 0 <= i < items.len() && items[i].0 == key),
    {
        let mapped = items.map(|i: int, pair: (String, T)| pair.0);
        if keys(items).contains(key) {
            let i = choose|i: int| 0 <= i < mapped.len() && mapped[i] == key;
            assert(items[i].0 == key);
        } else if exists|i: int| 0 <= i < items.len() && items[i].0 == key {
            let i = choose|i: int| 0 <= i < items.len() && items[i].0 == key;
            assert(mapped[i] == key);
        }
    }
    pub proof fn pending_pop<T>(frontier: crate::traversal::Frontier<T>)
        requires crate::traversal::valid(&frontier), frontier.pending@.len() > 0,
        ensures keys(frontier.pending@.drop_last()) == keys(frontier.pending@).remove(frontier.pending@.last().0),
            !keys(frontier.pending@.drop_last()).contains(frontier.pending@.last().0),
    {
        let all = frontier.pending@;
        keys_push(all.drop_last(), all.last().0, all.last().1);
        assert(all.drop_last().push(all.last()) == all);
        key_membership(all.drop_last(), all.last().0);
        if keys(all.drop_last()).contains(all.last().0) {
            let i = choose|i: int| 0 <= i < all.drop_last().len() && all.drop_last()[i].0 == all.last().0;
            assert(all[i].0 != all[all.len() - 1].0);
        }
        assert(keys(all.drop_last()) =~= keys(all).remove(all.last().0));
    }
    pub proof fn keys_first<T>(items: Seq<(String, T)>)
        requires items.len() > 0,
        ensures keys(items) == keys(items.drop_first()).insert(items[0].0),
    {
        assert forall|k: String| keys(items).contains(k) == keys(items.drop_first()).insert(items[0].0).contains(k) by {
            key_membership(items, k); key_membership(items.drop_first(), k);
            if keys(items).contains(k) {
                let i = choose|i: int| 0 <= i < items.len() && items[i].0 == k;
                if i > 0 { assert(items.drop_first()[i - 1].0 == k); }
            } else if keys(items.drop_first()).contains(k) {
                let i = choose|i: int| 0 <= i < items.drop_first().len() && items.drop_first()[i].0 == k;
                assert(items[i + 1].0 == k);
            }
        }
        assert(keys(items) =~= keys(items.drop_first()).insert(items[0].0));
    }
}

#[cfg(not(straw_verus))]
pub trait SourceModel {
    type Reference;
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Source: SourceModel {
    type Error;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires valid(old(self)@), old(self)@.nodes.contains_key(*key), (old(self)@.key)(*reference) == *key,
        ensures final(self)@ == old(self)@,
            result.is_ok() ==> result->Ok_0.pack == old(self)@.nodes[*key].pack
                && keys(result->Ok_0.children@) == old(self)@.nodes[*key].children
                && bound(old(self)@, result->Ok_0.children@),
    ))]
    #[allow(clippy::ptr_arg)]
    fn expand(
        &mut self,
        key: &String,
        reference: &Self::Reference,
    ) -> Result<Expansion<Self::Reference>, Self::Error>;
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures result@ == paths@,
))]
fn copy_paths(paths: &BTreeSet<String>) -> BTreeSet<String> {
    paths.clone()
}

#[cfg_attr(straw_verus, verus_spec(
    requires crate::environment::string_order_contract(),
    ensures final(result)@ == old(result)@.union(additions@),
))]
fn add_cached(result: &mut BTreeSet<String>, additions: &BTreeSet<String>) {
    let mut entries = additions.iter();
    #[cfg_attr(straw_verus, verus_spec(
        invariant entries.decrease().is_some(), crate::environment::string_order_contract(),
            result@.union(entries.remaining().unref().to_set()) == old(result)@.union(additions@),
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        #[cfg(straw_verus)]
        proof! { if entries.remaining().len() > 0 { crate::catalog_state::pop_set(entries.remaining()); } }
        let Some(path) = entries.next() else {
            return;
        };
        result.insert(path.clone());
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires valid(old(source)@), old(source)@.nodes.contains_key(root_key),
        (old(source)@.key)(root) == root_key, cached(old(source)@, old(cache)@),
        crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
    ensures final(source)@ == old(source)@, cached(final(source)@, final(cache)@),
        result.is_err() ==> final(cache)@ == old(cache)@,
        result.is_ok() ==> result->Ok_0@ == packs(old(source)@, root_key)
            && final(cache)@.contains_key(root_key) && final(cache)@[root_key]@ == result->Ok_0@,
))]
pub fn collect<S: Source>(
    source: &mut S,
    cache: &mut HashMap<String, BTreeSet<String>>,
    root_key: String,
    root: S::Reference,
) -> Result<BTreeSet<String>, S::Error> {
    if let Some(paths) = cache.get(&root_key) {
        return Ok(copy_paths(paths));
    }
    let mut paths = BTreeSet::new();
    let mut frontier = crate::traversal::start();
    crate::traversal::schedule(&mut frontier, root_key.clone(), root);
    #[cfg(straw_verus)]
    let mut visited = verus_exec_expr!(Ghost(Set::<String>::empty()));
    #[cfg(straw_verus)]
    proof! {
        root_reaches(source@, root_key);
        keys_push(Seq::<(String, S::Reference)>::empty(), root_key, root);
        assert(keys(Seq::<(String, S::Reference)>::empty()) =~= Set::<String>::empty());
        assert(keys(frontier.pending@) =~= Set::<String>::empty().insert(root_key));
    }
    #[cfg_attr(straw_verus, verus_spec(
        invariant source@ == old(source)@, cache@ == old(cache)@, valid(source@), cached(source@, cache@),
            source@.nodes.contains_key(root_key),
            crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
            crate::traversal::valid(&frontier), bound(source@, frontier.pending@),
            frontier.seen@.contains(root_key), frontier.seen@.subset_of(nodes(source@, root_key)),
            frontier.seen@ == visited@.union(keys(frontier.pending@)),
            visited@.disjoint(keys(frontier.pending@)),
            visited@.subset_of(source@.nodes.dom()),
            paths@.subset_of(packs(source@, root_key)), covered(source@, visited@, frontier.seen@, paths@),
        ensures paths@ == packs(source@, root_key),
        decreases source@.nodes.dom().difference(visited@).len(),
    ))]
    loop {
        #[cfg(straw_verus)]
        let before_frontier = verus_exec_expr!(Ghost(frontier));
        #[cfg(straw_verus)]
        let before_visited = verus_exec_expr!(Ghost(visited@));
        #[cfg(straw_verus)]
        proof! { if frontier.pending@.len() > 0 { pending_pop(frontier); } }
        let Some((key, reference)) = crate::traversal::next(&mut frontier) else {
            #[cfg(straw_verus)]
            proof! {
                assert(before_frontier@.pending@.len() == 0);
                assert(frontier.pending@ == before_frontier@.pending@);
                assert(keys(frontier.pending@) =~= Set::<String>::empty());
                assert(visited@.union(keys(frontier.pending@)) =~= visited@);
                assert(frontier.seen@ == visited@);
                complete(source@, root_key, frontier.seen@, paths@);
            }
            break;
        };
        #[cfg(straw_verus)]
        let before_paths = verus_exec_expr!(Ghost(paths@));
        #[cfg(straw_verus)]
        proof! {
            assert(before_frontier@.pending@.len() > 0);
            assert(key == before_frontier@.pending@.last().0);
            key_membership(before_frontier@.pending@, key);
            assert(keys(before_frontier@.pending@).contains(key));
            assert(!visited@.contains(key));
            assert(frontier.seen@ == visited@.union(keys(frontier.pending@)).insert(key));
            assert(!keys(frontier.pending@).contains(key));
            assert(nodes(source@, root_key).contains(key));
            subclosure(source@, root_key, key);
        }
        if let Some(cached_paths) = cache.get(&key) {
            add_cached(&mut paths, cached_paths);
            #[cfg(straw_verus)]
            proof! { coverage_grows(source@, visited@, frontier.seen@, before_paths@, frontier.seen@, paths@); }
        } else {
            let expansion = source.expand(&key, &reference)?;
            paths.insert(expansion.pack);
            #[cfg(straw_verus)]
            proof! {
                root_reaches(source@, key);
                coverage_grows(source@, visited@, frontier.seen@, before_paths@, frontier.seen@, paths@);
            }
            let mut children = expansion.children.into_iter();
            #[cfg_attr(straw_verus, verus_spec(
                invariant source@ == old(source)@, cache@ == old(cache)@, valid(source@), cached(source@, cache@),
                    crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
                    children.decrease().is_some(), bound(source@, children.remaining()),
                    keys(children.remaining()).subset_of(source@.nodes[key].children),
                    source@.nodes[key].children.subset_of(frontier.seen@.union(keys(children.remaining()))),
                    crate::traversal::valid(&frontier), bound(source@, frontier.pending@),
                    frontier.seen@.contains(root_key), frontier.seen@.subset_of(nodes(source@, root_key)),
                    frontier.seen@ == visited@.union(keys(frontier.pending@)).insert(key),
                    visited@.disjoint(keys(frontier.pending@)), !visited@.contains(key), !keys(frontier.pending@).contains(key),
                    nodes(source@, root_key).contains(key), paths@.contains(source@.nodes[key].pack),
                    visited@ == before_visited@,
                    paths@.subset_of(packs(source@, root_key)), covered(source@, visited@, frontier.seen@, paths@),
                ensures children.remaining().len() == 0,
                decreases children.decrease().unwrap(),
            ))]
            loop {
                #[cfg(straw_verus)]
                proof! { if children.remaining().len() > 0 { keys_first(children.remaining()); } }
                let Some((child_key, child)) = children.next() else {
                    break;
                };
                #[cfg(straw_verus)]
                let before_seen = verus_exec_expr!(Ghost(frontier.seen@));
                #[cfg(straw_verus)]
                proof! {
                    append_edge(source@, root_key, key, child_key);
                    keys_push(frontier.pending@, child_key, child);
                }
                crate::traversal::schedule(&mut frontier, child_key, child);
                #[cfg(straw_verus)]
                proof! { coverage_grows(source@, visited@, before_seen@, paths@, frontier.seen@, paths@); }
            }
            #[cfg(straw_verus)]
            proof! {
                assert(keys(children.remaining()) =~= Set::<String>::empty());
                assert(frontier.seen@.union(keys(children.remaining())) =~= frontier.seen@);
                assert(source@.nodes[key].children.subset_of(frontier.seen@));
            }
        }
        #[cfg(straw_verus)]
        proof! {
            assert(covered(source@, visited@.insert(key), frontier.seen@, paths@));
            let more = visited@.insert(key);
            assert(!visited@.contains(key));
            source@.nodes.dom().lemma_set_insert_diff_decreases(visited@, key);
            assert(more.len() == before_visited@.len() + 1);
            *visited.borrow_mut() = more;
        }
    }
    #[cfg(straw_verus)]
    proof! {
        assert(paths@ == packs(source@, root_key));
        assert(source@ == old(source)@);
        assert(cached(source@, cache@));
    }
    // Cache only complete requested roots; intermediate suffixes can be quadratic.
    if cache.len() > 100000 {
        cache.clear();
    }
    #[cfg(straw_verus)]
    let previous = verus_exec_expr!(Ghost(cache@));
    #[cfg(straw_verus)]
    proof! { assert(cached(source@, previous@)); }
    let copied = copy_paths(&paths);
    cache.insert(root_key, copied);
    #[cfg(straw_verus)]
    proof! {
        assert(source@.nodes.contains_key(root_key));
        assert(cache@ == previous@.insert(root_key, copied));
        assert forall|k: String| cache@.contains_key(k) implies source@.nodes.contains_key(k) && cache@[k]@ == packs(source@, k) by {
            if k != root_key {
                assert(previous@.contains_key(k));
                assert(cache@[k] == previous@[k]);
                assert(previous@[k]@ == packs(source@, k));
            } else {
                assert(cache@[k] == copied);
                assert(copied@ == paths@);
                assert(paths@ == packs(source@, root_key));
            }
        }
        assert(cached(source@, cache@));
    }
    Ok(paths)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
    ensures result == !paths@.disjoint(deleted@),
))]
pub(crate) fn reclaimed(paths: &BTreeSet<String>, deleted: &HashSet<String>) -> bool {
    let mut entries = paths.iter();
    #[cfg_attr(straw_verus, verus_spec(
        invariant entries.decrease().is_some(), crate::environment::string_hash_contract(),
            entries.remaining().unref().to_set().subset_of(paths@),
            entries.remaining().unref().to_set().intersect(deleted@) == paths@.intersect(deleted@),
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        #[cfg(straw_verus)]
        proof! { if entries.remaining().len() > 0 { crate::catalog_state::pop_set(entries.remaining()); } }
        let Some(path) = entries.next() else {
            #[cfg(straw_verus)]
            proof! {
                assert(entries.remaining().unref().to_set() =~= Set::<String>::empty());
                assert(paths@.intersect(deleted@) =~= Set::<String>::empty());
                assert forall|p: String| paths@.contains(p) implies !deleted@.contains(p) by {
                    if deleted@.contains(p) { assert(paths@.intersect(deleted@).contains(p)); }
                }
            }
            return false;
        };
        if deleted.contains(path) {
            #[cfg(straw_verus)]
            proof! { assert(paths@.contains(*path)); }
            return true;
        }
    }
}

/// Snapshot validation before a caller creates ownership; it does not itself retain.
#[cfg_attr(straw_verus, verus_spec(result =>
    requires valid(old(source)@), old(source)@.nodes.contains_key(root_key),
        (old(source)@.key)(root) == root_key, cached(old(source)@, old(cache)@),
        crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
    ensures final(source)@ == old(source)@, cached(final(source)@, final(cache)@),
        match result {
            Err(Failure::Read(_)) => final(cache)@ == old(cache)@,
            Err(Failure::Reclaimed) => !packs(old(source)@, root_key).disjoint(deleted@),
            Ok(paths) => paths@ == packs(old(source)@, root_key) && paths@.disjoint(deleted@),
        },
))]
pub fn resolve<S: Source>(
    source: &mut S,
    cache: &mut HashMap<String, BTreeSet<String>>,
    deleted: &HashSet<String>,
    root_key: String,
    root: S::Reference,
) -> Result<BTreeSet<String>, Failure<S::Error>> {
    let paths = match collect(source, cache, root_key, root) {
        Ok(paths) => paths,
        Err(error) => return Err(Failure::Read(error)),
    };
    if reclaimed(&paths, deleted) {
        return Err(Failure::Reclaimed);
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Reader {
        nodes: HashMap<String, (String, Vec<String>)>,
        calls: Vec<String>,
        failure: Option<String>,
    }
    impl SourceModel for Reader {
        type Reference = String;
    }
    impl Source for Reader {
        type Error = &'static str;
        fn expand(
            &mut self,
            key: &String,
            reference: &String,
        ) -> Result<Expansion<String>, Self::Error> {
            assert_eq!(key, reference);
            self.calls.push(key.clone());
            if self.failure.as_ref() == Some(key) {
                return Err("injected read failure");
            }
            let (pack, children) = self.nodes.get(key).ok_or("unknown node")?;
            Ok(Expansion {
                pack: pack.clone(),
                children: children
                    .iter()
                    .map(|key| (key.clone(), key.clone()))
                    .collect(),
            })
        }
    }
    fn reader(nodes: &[(&str, &str, &[&str])]) -> Reader {
        Reader {
            nodes: nodes
                .iter()
                .map(|(k, pack, children)| {
                    (
                        k.to_string(),
                        (
                            pack.to_string(),
                            children.iter().map(|s| s.to_string()).collect(),
                        ),
                    )
                })
                .collect(),
            calls: Vec::new(),
            failure: None,
        }
    }
    fn expected(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn cycle_repeated_edges_and_shared_pack_are_complete() {
        let mut source = reader(&[
            ("root", "root.pack", &["a", "a", "b"]),
            ("a", "shared.pack", &["root", "leaf"]),
            ("b", "shared.pack", &["leaf"]),
            ("leaf", "leaf.pack", &["leaf"]),
        ]);
        let mut cache = HashMap::new();
        let result = collect(&mut source, &mut cache, "root".into(), "root".into()).unwrap();
        assert_eq!(result, expected(&["root.pack", "shared.pack", "leaf.pack"]));
        source.calls.sort();
        assert_eq!(source.calls, ["a", "b", "leaf", "root"]);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache["root"], result);
        source.calls.clear();
        source.failure = Some("root".into());
        assert_eq!(
            collect(&mut source, &mut cache, "root".into(), "root".into()).unwrap(),
            result
        );
        assert!(source.calls.is_empty());
    }

    #[test]
    fn failed_traversal_does_not_poison_cached_subclosure() {
        let mut source = reader(&[
            ("root", "root.pack", &["a", "b"]),
            ("a", "a.pack", &["leaf"]),
            ("b", "b.pack", &["leaf"]),
            ("leaf", "leaf.pack", &[]),
        ]);
        let mut cache = HashMap::new();
        assert_eq!(
            collect(&mut source, &mut cache, "a".into(), "a".into()).unwrap(),
            expected(&["a.pack", "leaf.pack"])
        );
        let before = cache.clone();
        source.failure = Some("leaf".into());
        assert!(collect(&mut source, &mut cache, "root".into(), "root".into()).is_err());
        assert_eq!(cache, before);
        source.failure = None;
        source.calls.clear();
        let result = collect(&mut source, &mut cache, "root".into(), "root".into()).unwrap();
        assert_eq!(
            result,
            expected(&["root.pack", "a.pack", "b.pack", "leaf.pack"])
        );
        assert!(!source.calls.contains(&"a".to_string()));
        assert_eq!(cache["a"], before["a"]);
        assert_eq!(cache["root"], result);
    }

    #[test]
    fn transitive_tombstone_rejects_cold_and_cached_results() {
        let mut source = reader(&[("root", "root.pack", &["leaf"]), ("leaf", "leaf.pack", &[])]);
        let mut cache = HashMap::new();
        let deleted = HashSet::from(["leaf.pack".to_string()]);
        assert!(matches!(
            resolve(
                &mut source,
                &mut cache,
                &deleted,
                "root".into(),
                "root".into()
            ),
            Err(Failure::Reclaimed)
        ));
        assert_eq!(cache["root"], expected(&["root.pack", "leaf.pack"]));
        source.calls.clear();
        source.failure = Some("root".into());
        assert!(matches!(
            resolve(
                &mut source,
                &mut cache,
                &deleted,
                "root".into(),
                "root".into()
            ),
            Err(Failure::Reclaimed)
        ));
        assert!(source.calls.is_empty());
    }
}
