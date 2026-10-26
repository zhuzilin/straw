//! Construct catalog operations from decoded fields and namespace identities.
//! The resolver contract is a separate native namespace-adapter obligation.
use crate::catalog_state::Operation;
use crate::identities::Encoding;
use std::collections::{BTreeSet, HashMap};

pub trait Resolver {
    type Error;
    /// Clear derived lookup caches without changing the namespace they resolve.
    fn reset(&mut self) {}

    // Borrow the decoded String key used by the namespace model; no new allocation.
    #[allow(clippy::ptr_arg)]
    fn resolve(&mut self, name: &str, encoding: Encoding) -> Result<String, Self::Error>;
}

fn resolve_paths<R: Resolver>(
    resolver: &mut R,
    names: &[String],
    encoding: Encoding,
) -> Result<BTreeSet<String>, R::Error> {
    let mut result = Vec::with_capacity(names.len());
    let mut i = 0usize;

    while i < names.len() {
        let path = resolver.resolve(&names[i], encoding)?;

        result.push(path);
        i += 1;
    }

    Ok(result.into_iter().collect())
}

pub fn append_owners<R: Resolver>(
    resolver: &mut R,
    owners: crate::catalog_json::OwnerList,
    encoding: Encoding,
    operations: &mut Vec<Operation>,
) -> Result<(), R::Error> {
    let mut entries = owners.into_iter();

    loop {
        let Some((owner, names)) = entries.next() else {
            return Ok(());
        };
        let op = match names {
            None => Operation::Release(owner),
            Some(names) => Operation::Owner(owner, resolve_paths(resolver, &names, encoding)?),
        };

        operations.push(op);
    }
}

fn append_packs<R: Resolver>(
    resolver: &mut R,
    packs: Vec<(String, bool)>,
    encoding: Encoding,
    operations: &mut Vec<Operation>,
) -> Result<(), R::Error> {
    let mut entries = packs.into_iter();

    loop {
        let Some((path, sealed)) = entries.next() else {
            return Ok(());
        };
        let key = resolver.resolve(&path, encoding)?;
        let op = Operation::Pack(key, sealed);

        operations.push(op);
    }
}

fn append_deletions<R: Resolver>(
    resolver: &mut R,
    names: Vec<String>,
    encoding: Encoding,
    reclaimed: bool,
    operations: &mut Vec<Operation>,
) -> Result<(), R::Error> {
    let mut entries = names.into_iter();

    loop {
        let Some(path) = entries.next() else {
            return Ok(());
        };
        let key = resolver.resolve(&path, encoding)?;
        let op = if reclaimed {
            Operation::Reclaim(key)
        } else {
            Operation::Mark(key)
        };

        operations.push(op);
    }
}

fn graph(edges: crate::catalog_json::OwnerList) -> Result<HashMap<String, Vec<String>>, ()> {
    let mut result = HashMap::new();
    let mut entries = edges.into_iter();

    loop {
        let Some((key, children)) = entries.next() else {
            return Ok(result);
        };
        let Some(children) = children else {
            return Err(());
        };

        result.insert(key, children);
    }
}

fn graph_operations(additions: HashMap<String, Vec<String>>) -> Vec<Operation> {
    let mut entries = additions.into_iter();

    let mut operations = Vec::new();

    loop {
        let Some((key, children)) = entries.next() else {
            return operations;
        };
        let op = Operation::Edges(key, children);

        operations.push(op);
    }
}

#[derive(Debug)]
pub enum Failure<E> {
    Wire(crate::catalog_json::DecodeError),
    Receipts,
    Graph,
    Unsupported,
    Path(E),
}

fn construct<R: Resolver>(
    resolver: &mut R,
    wire: crate::catalog_json::Wire,
    known: &HashMap<String, Vec<String>>,
) -> Result<Vec<Operation>, Failure<R::Error>> {
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

    if matches!(encoding, Encoding::Unsupported) {
        return Err(Failure::Unsupported);
    }
    let mut operations = graph_operations(additions);

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

    operations.push(receipts);

    Ok(operations)
}

pub fn decode<R: Resolver>(
    resolver: &mut R,
    value: &serde_json::Value,
    known: &HashMap<String, Vec<String>>,
) -> Result<Vec<Operation>, Failure<R::Error>> {
    let wire = match crate::catalog_json::decode(value) {
        Ok(wire) => wire,
        Err(error) => return Err(Failure::Wire(error)),
    };

    construct(resolver, wire, known)
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

        fn resolve(&mut self, name: &str, encoding: Encoding) -> Result<String, Self::Error> {
            self.calls.push(name.to_owned());
            match encoding {
                Encoding::Resolved => Ok(name.to_owned()),
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
