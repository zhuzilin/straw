//! Actual queue GC retirement barrier, owner naming and locked catalog dispatch.
//! The retention token, exact owner set and catalog append are checked code.
//! Filesystem durability, lock refresh and closure identity remain separate contracts.
#[cfg(test)]
use crate::queue_roots::Roots;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub use crate::queue_retirement::Retired;

pub struct Prepared<'a> {
    prefix: &'a str,
    live: HashSet<String>,
}

fn owners(prefix: &str, roots: &HashMap<String, Value>) -> HashSet<String> {
    let mut live = HashSet::new();
    let mut entries = roots.iter();

    loop {
        let Some((key, _)) = entries.next() else {
            return live;
        };

        let mut owner = prefix.to_owned();
        owner.push_str(key);

        live.insert(owner);
    }
}

pub fn prepare<'a>(prefix: &'a str, retired: Retired) -> Prepared<'a> {
    let roots = retired.into_roots();
    let live = owners(prefix, &roots.references);
    Prepared { prefix, live }
}

impl<'a> Prepared<'a> {
    pub(crate) fn parts(self) -> (&'a str, HashSet<String>) {
        (self.prefix, self.live)
    }
}

pub(crate) fn not_publication(name: &str) -> bool {
    !name.as_bytes().starts_with("publication:".as_bytes())
}

pub(crate) fn not_publication_owners(names: &[String]) -> bool {
    let mut i = 0usize;

    while i < names.len() {
        if !not_publication(&names[i]) {
            return false;
        }
        i += 1;
    }
    true
}

pub(crate) fn public_owner(name: &str) -> bool {
    not_publication(name) && !name.as_bytes().starts_with("queue:".as_bytes())
}

pub fn prunable(
    owners: &HashMap<String, std::collections::BTreeSet<String>>,
    prefix: &str,
    live: &HashSet<String>,
) -> Vec<String> {
    let mut result = Vec::new();
    let mut entries = owners.iter();

    loop {
        let Some((owner, _)) = entries.next() else {
            return result;
        };
        let discard =
            owner.as_str().as_bytes().starts_with(prefix.as_bytes()) && !live.contains(owner);

        if discard {
            result.push(owner.clone());
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnershipError {
    Missing,
    Invalid,
}

pub fn require(
    owners: &HashMap<String, std::collections::BTreeSet<String>>,
    deleted: &HashSet<String>,
    live: &HashSet<String>,
) -> Result<(), OwnershipError> {
    let mut entries = live.iter();

    loop {
        let Some(owner) = entries.next() else {
            return Ok(());
        };

        let Some(paths) = owners.get(owner) else {
            return Err(OwnershipError::Missing);
        };
        if paths.is_empty() || crate::closure::reclaimed(paths, deleted) {
            return Err(OwnershipError::Invalid);
        }
    }
}

fn release_value(
    known: &HashMap<String, std::collections::BTreeSet<String>>,
    names: &[String],
) -> Option<Value> {
    let mut updates = serde_json::Map::new();
    let mut any = false;
    let mut i = 0usize;

    while i < names.len() {
        if known.contains_key(&names[i]) {
            updates.insert(names[i].clone(), Value::Null);
            any = true;
        }
        i += 1;
    }

    if !any {
        return None;
    }
    let mut object = serde_json::Map::new();

    object.insert("owners".to_owned(), Value::Object(updates));
    object.insert("path_encoding".to_owned(), Value::Number(1u64.into()));
    Some(Value::Object(object))
}

pub fn release<R: crate::catalog_decode::Resolver, D: crate::wal_device::Writer>(
    log: &mut crate::catalog_log::LogState,
    resolver: &mut R,
    run: &str,
    names: &[String],
    device: &mut D,
) -> Result<(), crate::catalog_append::Error<R::Error, D::Error>> {
    let indexes = log.indexes();

    let value = release_value(&indexes.owners, names);

    let Some(value) = value else {
        return Ok(());
    };
    crate::catalog_append::append(log, resolver, run, value, device)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    fn roots() -> Roots {
        Roots {
            references: HashMap::from([("one".into(), Value::Null), ("二".into(), Value::Null)]),
            prefix: 3,
            positions: HashSet::from([4, 7]),
        }
    }
    #[test]
    fn prepared_roots_preserve_exact_owner_names() {
        let retired = Retired::fixture(roots());
        let prepared = prepare("queue:a:", retired);
        let (prefix, live) = prepared.parts();
        assert_eq!(prefix, "queue:a:");
        assert_eq!(
            live,
            HashSet::from(["queue:a:one".into(), "queue:a:二".into()])
        );
    }
    #[test]
    fn pruning_preserves_other_queues_staging_and_current_roots() {
        let owners = [
            "queue:a:old",
            "queue:a:live",
            "queue:ab:peer",
            "queue:b:peer",
            "staged:publication",
            "writer:open",
            "队列:旧",
        ]
        .into_iter()
        .map(|name| (name.to_owned(), BTreeSet::from(["pack".to_owned()])))
        .collect::<HashMap<_, _>>();
        let live = HashSet::from(["queue:a:live".to_owned()]);
        assert_eq!(
            prunable(&owners, "queue:a:", &live),
            vec!["queue:a:old".to_owned()]
        );
        assert_eq!(
            prunable(&owners, "队列:", &live),
            vec!["队列:旧".to_owned()]
        );
        assert!(prunable(&owners, "queue:absent:", &live).is_empty());
        let expected = owners
            .keys()
            .filter(|name| !live.contains(*name))
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            prunable(&owners, "", &live)
                .into_iter()
                .collect::<BTreeSet<_>>(),
            expected
        );
    }
    #[test]
    fn release_event_preserves_wire_format_and_filters_unknown_or_duplicate_names() {
        let known = HashMap::from([
            ("old".to_owned(), BTreeSet::from(["pack".to_owned()])),
            ("peer".to_owned(), BTreeSet::from(["shared".to_owned()])),
        ]);
        assert!(release_value(&known, &["absent".to_owned()]).is_none());
        assert!(release_value(&known, &[]).is_none());
        let event = release_value(
            &known,
            &["absent".to_owned(), "old".to_owned(), "old".to_owned()],
        )
        .unwrap();
        assert_eq!(
            event,
            serde_json::json!({"owners":{"old":null},"path_encoding":1})
        );
    }

    #[test]
    fn retained_check_rejects_absent_empty_and_deleted_owner_paths() {
        let mut owners = HashMap::from([("live".to_owned(), BTreeSet::from(["pack".to_owned()]))]);
        let live = HashSet::from(["live".to_owned()]);
        assert_eq!(require(&owners, &HashSet::new(), &live), Ok(()));
        assert_eq!(
            require(&owners, &HashSet::from(["pack".to_owned()]), &live),
            Err(OwnershipError::Invalid)
        );
        owners.insert("live".to_owned(), BTreeSet::new());
        assert_eq!(
            require(&owners, &HashSet::new(), &live),
            Err(OwnershipError::Invalid)
        );
        owners.remove("live");
        assert_eq!(
            require(&owners, &HashSet::new(), &live),
            Err(OwnershipError::Missing)
        );
        assert_eq!(require(&owners, &HashSet::new(), &HashSet::new()), Ok(()));
    }
}
