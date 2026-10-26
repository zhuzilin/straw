//! The actual staging recipe traversal, including persisted recipes after GC.
use crate::closure_node::{Digest, Kind};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub struct Plan {
    pub nodes: HashSet<String>,
    pub raw: HashSet<String>,
    pub additions: HashMap<String, Vec<String>>,
}

pub enum Error<R, D> {
    Read(R),
    Digest(D),
    Reference,
    Missing,
    Dependencies,
    Records,
}

pub trait Source: crate::closure::SourceModel<Reference = Value> {
    type Error;

    fn manifest(&mut self, reference: &Value) -> Result<Value, Self::Error>;

    fn point(&mut self, name: &str) -> Result<(), Self::Error>;
}

pub fn name<D: Digest>(digester: &mut D, value: &Value) -> Result<String, D::Error> {
    let key = digester.digest(value)?;
    let mut result = "staged:".to_owned();
    result.push_str(&key);
    Ok(result)
}

fn known_children(frontier: &mut crate::traversal::Frontier<Option<Value>>, children: &[String]) {
    let mut i = 0usize;

    while i < children.len() {
        crate::traversal::schedule(frontier, children[i].clone(), None);
        i += 1;
    }
}

fn fresh_children<D: Digest>(
    digester: &mut D,
    frontier: &mut crate::traversal::Frontier<Option<Value>>,
    children: &mut Vec<String>,
    values: &[Value],
) -> Result<(), D::Error> {
    let mut i = 0usize;

    while i < values.len() {
        let key = name(digester, &values[i])?;

        children.push(key.clone());
        let value = Some(values[i].clone());

        crate::traversal::schedule(frontier, key, value);
        i += 1;
    }

    Ok(())
}

// Keep the reader and digest failures distinct in the checked traversal contract.
#[allow(clippy::type_complexity)]
fn expand<S: Source, D: Digest>(
    source: &mut S,
    digester: &mut D,
    known: &HashMap<String, Vec<String>>,
    frontier: &mut crate::traversal::Frontier<Option<Value>>,
    key: &String,
    reference: Option<Value>,
) -> Result<Option<Vec<String>>, Error<S::Error, D::Error>> {
    if let Some(children) = known.get(key) {
        known_children(frontier, children);

        return Ok(None);
    }
    let Some(reference) = reference else {
        return Err(Error::Missing);
    };
    let mut children = Vec::new();
    match crate::closure_node::kind(&reference) {
        Kind::Manifest => {
            if let Err(e) = source.point("before_staging_manifest_read") {
                return Err(Error::Read(e));
            }
            let manifest = match source.manifest(&reference) {
                Ok(m) => m,
                Err(e) => return Err(Error::Read(e)),
            };
            if let Err(e) = source.point("after_staging_manifest_read") {
                return Err(Error::Read(e));
            }
            let Some(dependencies) =
                crate::catalog_json::get(&manifest, "dependencies").and_then(Value::as_array)
            else {
                return Err(Error::Dependencies);
            };
            let Some(records) =
                crate::catalog_json::get(&manifest, "records").and_then(Value::as_array)
            else {
                return Err(Error::Records);
            };
            if let Err(e) = fresh_children(digester, frontier, &mut children, dependencies) {
                return Err(Error::Digest(e));
            }
            if let Err(e) = fresh_children(digester, frontier, &mut children, records) {
                return Err(Error::Digest(e));
            }
        }
        Kind::Record => {}
        Kind::Invalid => return Err(Error::Reference),
    }

    Ok(Some(children))
}

pub fn plan<S: Source, D: Digest>(
    source: &mut S,
    digester: &mut D,
    known: &HashMap<String, Vec<String>>,
    references: &[Value],
) -> Result<Plan, Error<S::Error, D::Error>> {
    let mut frontier = crate::traversal::start();
    let mut raw = HashSet::new();
    let mut i = 0usize;

    while i < references.len() {
        let reference = &references[i];
        let kind = crate::closure_node::kind(reference);
        if let Kind::Invalid = kind {
            return Err(Error::Reference);
        }
        let key = match name(digester, reference) {
            Ok(k) => k,
            Err(e) => return Err(Error::Digest(e)),
        };

        if let Kind::Manifest = kind {
            let value = Some(reference.clone());

            crate::traversal::schedule(&mut frontier, key, value);
        } else {
            raw.insert(key);
        }

        i += 1;
    }

    let mut additions = HashMap::<String, Vec<String>>::new();

    loop {
        let Some((key, reference)) = crate::traversal::next(&mut frontier) else {
            return Ok(Plan {
                nodes: frontier.seen,
                raw,
                additions,
            });
        };

        let children = expand(source, digester, known, &mut frontier, &key, reference)?;
        if let Some(children) = children {
            additions.insert(key.clone(), children);
        }
    }
}

fn owned_names<K>(names: HashSet<K>) -> std::collections::hash_set::IntoIter<K> {
    names.into_iter()
}

fn owner_updates(
    names: HashSet<String>,
    known: &HashMap<String, std::collections::BTreeSet<String>>,
    updates: &mut serde_json::Map<String, Value>,
) -> bool {
    let mut entries = owned_names(names);
    let mut any = false;

    loop {
        let Some(owner) = entries.next() else {
            return any;
        };

        if known.contains_key(&owner) {
            updates.insert(owner, Value::Null);
            any = true;
        }
    }
}

fn string_array(values: Vec<String>) -> Value {
    let mut entries = values.into_iter();
    let mut output = Vec::new();

    loop {
        let Some(name) = entries.next() else {
            return Value::Array(output);
        };
        output.push(Value::String(name));
    }
}

fn edge_values(edges: HashMap<String, Vec<String>>) -> Value {
    let mut entries = edges.into_iter();
    let mut object = serde_json::Map::new();

    loop {
        let Some((key, children)) = entries.next() else {
            return Value::Object(object);
        };

        object.insert(key.clone(), string_array(children));
    }
}

pub fn value(
    plan: Plan,
    known: &HashMap<String, std::collections::BTreeSet<String>>,
    receipt: Option<&str>,
) -> Option<Value> {
    let mut updates = serde_json::Map::new();

    let first = owner_updates(plan.nodes, known, &mut updates);
    let second = owner_updates(plan.raw, known, &mut updates);

    if !first && !second && plan.additions.is_empty() && receipt.is_none() {
        return None;
    }
    let mut object = serde_json::Map::new();

    object.insert("owners".to_owned(), Value::Object(updates));
    if !plan.additions.is_empty() {
        object.insert("staging_edges".to_owned(), edge_values(plan.additions));
    }
    if let Some(receipt) = receipt {
        object.insert(
            "consumed_handoffs".to_owned(),
            Value::Array(vec![Value::String(receipt.to_owned())]),
        );
    }
    object.insert("path_encoding".to_owned(), Value::Number(1u64.into()));
    Some(Value::Object(object))
}
