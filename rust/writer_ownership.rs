//! Production preparation-owner coverage and exact writer-directory sealing.
use crate::writer_registry::Operation;
use serde_json::Value;

pub struct Held {
    pub owner: String,
    pub paths: Vec<String>,
}

pub trait Coverage {
    fn reset(&mut self);

    fn include(&mut self, dependency: &Value);

    fn contains(&self, path: &str) -> bool;
}

fn registered_owner(operations: &[Operation], owner: &str) -> bool {
    let mut index = 0;

    while index < operations.len() {
        if crate::writer_retirement::named_operation(&operations[index].evidence, owner, true) {
            return true;
        }
        index += 1;
    }
    false
}

fn certified_owner<C: Coverage>(
    io: &mut C,
    operations: &[Operation],
    owner: &str,
    paths: &[String],
) -> bool {
    io.reset();
    let mut index = 0;

    while index < operations.len() {
        if crate::writer_retirement::named_operation(&operations[index].evidence, owner, true) {
            let dependencies = &operations[index].evidence.dependencies;
            let mut dependency = 0;

            while dependency < dependencies.len() {
                io.include(&dependencies[dependency]);

                dependency += 1;
            }
        }
        index += 1;
    }
    let mut path = 0;

    while path < paths.len() {
        if !io.contains(&paths[path]) {
            return false;
        }
        path += 1;
    }
    true
}

pub(crate) fn starts_with(value: &[u8], prefix: &[u8]) -> bool {
    if value.len() < prefix.len() {
        return false;
    }
    let mut index = 0;

    while index < prefix.len() {
        if value[index] != prefix[index] {
            return false;
        }
        index += 1;
    }
    true
}

pub enum Failure {
    Unregistered,
    Uncertified,
}

// Prefix matching uses UTF-8 bytes. The loop's public contract uses the same
// actual String byte view, rather than pretending arbitrary Unicode is ASCII.

pub fn validate_owners<C: Coverage>(
    io: &mut C,
    operations: &[Operation],
    owners: &[Held],
    prefix: &[u8],
) -> Result<(), Failure> {
    let mut index = 0;

    while index < owners.len() {
        let held = &owners[index];
        if starts_with(held.owner.as_bytes(), prefix) {
            if !registered_owner(operations, &held.owner) {
                return Err(Failure::Unregistered);
            }
            if !certified_owner(io, operations, &held.owner, &held.paths) {
                return Err(Failure::Uncertified);
            }
        }
        index += 1;
    }
    Ok(())
}

pub trait ParentModel {
    type Identity;
}

pub trait Parents: ParentModel {
    type Error;

    fn canonical_parent(&mut self, path: &str) -> Result<Option<Self::Identity>, Self::Error>;

    fn same_parent(&self, left: &Self::Identity, right: &Self::Identity) -> bool;
}

pub fn seal_candidates<P: Parents>(
    io: &mut P,
    packs: &[(String, bool)],
    writer: &P::Identity,
) -> Result<Vec<usize>, P::Error> {
    let mut selected = Vec::new();
    let mut index = 0;

    while index < packs.len() {
        if !packs[index].1 {
            let parent = io.canonical_parent(&packs[index].0)?;
            if let Some(parent) = parent
                && io.same_parent(&parent, writer)
            {
                selected.push(index);
            }
        }

        index += 1;
    }
    Ok(selected)
}

pub fn result_dependencies(dependencies: &[Value]) -> bool {
    let mut index = 0;

    while index < dependencies.len() {
        if !matches!(
            crate::closure_node::kind(&dependencies[index]),
            crate::closure_node::Kind::Manifest
        ) {
            return false;
        }
        index += 1;
    }
    true
}

pub enum AuthenticationFailure<R, D> {
    Read(R),
    Digest(D),
}

#[allow(
    clippy::type_complexity,
    reason = "Keep component error types visible at the checked I/O boundary."
)]
pub fn authenticate_all<
    S: crate::closure::Source<Reference = Value>,
    D: crate::closure_node::Digest,
>(
    source: &mut S,
    digester: &mut D,
    roots: &[Value],
) -> Result<Vec<std::collections::BTreeSet<String>>, AuthenticationFailure<S::Error, D::Error>> {
    let mut cache = std::collections::HashMap::new();
    let mut authenticated: Vec<std::collections::BTreeSet<String>> = Vec::new();
    let mut index = 0;

    while index < roots.len() {
        let key = match digester.digest(&roots[index]) {
            Ok(key) => key,
            Err(error) => return Err(AuthenticationFailure::Digest(error)),
        };
        let root = roots[index].clone();
        let visited = match crate::closure::collect(source, &mut cache, key, root) {
            Ok(visited) => visited,
            Err(error) => return Err(AuthenticationFailure::Read(error)),
        };
        authenticated.push(visited);
        index += 1;
    }
    Ok(authenticated)
}
