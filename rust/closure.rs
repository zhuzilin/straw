//! Complete pack reachability, including cached subgraphs and cyclic edges.
//! Resolve immutable nodes into complete transitive pack closures before caching.
use std::collections::{BTreeSet, HashMap, HashSet};

pub struct Expansion<T> {
    pub pack: String,
    pub children: Vec<(String, T)>,
}

pub enum Failure<E> {
    Read(E),
    Reclaimed,
}

pub trait SourceModel {
    type Reference;
}

pub trait Source: SourceModel {
    type Error;

    #[allow(clippy::ptr_arg)]
    fn expand(
        &mut self,
        key: &String,
        reference: &Self::Reference,
    ) -> Result<Expansion<Self::Reference>, Self::Error>;
}

fn copy_paths(paths: &BTreeSet<String>) -> BTreeSet<String> {
    paths.clone()
}

fn add_cached(result: &mut BTreeSet<String>, additions: &BTreeSet<String>) {
    let mut entries = additions.iter();

    loop {
        let Some(path) = entries.next() else {
            return;
        };
        result.insert(path.clone());
    }
}

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

    loop {
        let Some((key, reference)) = crate::traversal::next(&mut frontier) else {
            break;
        };

        if let Some(cached_paths) = cache.get(&key) {
            add_cached(&mut paths, cached_paths);
        } else {
            let expansion = source.expand(&key, &reference)?;
            paths.insert(expansion.pack);

            let mut children = expansion.children.into_iter();

            loop {
                let Some((child_key, child)) = children.next() else {
                    break;
                };

                crate::traversal::schedule(&mut frontier, child_key, child);
            }
        }
    }

    // Cache only complete requested roots; intermediate suffixes can be quadratic.
    if cache.len() > 100000 {
        cache.clear();
    }

    let copied = copy_paths(&paths);
    cache.insert(root_key, copied);

    Ok(paths)
}

pub(crate) fn reclaimed(paths: &BTreeSet<String>, deleted: &HashSet<String>) -> bool {
    let mut entries = paths.iter();

    loop {
        let Some(path) = entries.next() else {
            return false;
        };
        if deleted.contains(path) {
            return true;
        }
    }
}

/// Snapshot validation before a caller creates ownership; it does not itself retain.
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
