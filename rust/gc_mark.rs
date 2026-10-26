//! Discover every unowned sealed pack and durably mark the same identities.
//! Input paths come from recovered catalog indexes, already in resolved form.
use crate::catalog_decode::Resolver;
use crate::catalog_log::LogState;
use serde_json::Value;
use std::collections::HashSet;

pub struct Marked {
    pub live: HashSet<String>,
    pub targets: Vec<String>,
}

fn snapshot(paths: &HashSet<String>) -> Vec<String> {
    let mut result = Vec::with_capacity(paths.len());
    let mut entries = paths.iter();

    loop {
        let Some(path) = entries.next() else {
            return result;
        };

        result.push(path.clone());
    }
}

fn event(paths: &[String]) -> Value {
    let mut values = Vec::with_capacity(paths.len());
    let mut i = 0usize;

    while i < paths.len() {
        values.push(Value::String(paths[i].clone()));
        i += 1;
    }

    let mut object = serde_json::Map::new();

    object.insert("deleted".to_owned(), Value::Array(values));
    object.insert("path_encoding".to_owned(), Value::Number(1u64.into()));
    Value::Object(object)
}

pub fn mark<R: Resolver, D: crate::wal_device::Writer>(
    log: &mut LogState,
    resolver: &mut R,
    run: &str,
    device: &mut D,
) -> Result<Marked, crate::catalog_append::Error<R::Error, D::Error>> {
    let live = log.indexes().roots();
    let candidates = log.indexes().candidates(&live);

    if !candidates.is_empty() {
        let paths = snapshot(&candidates);
        let value = event(&paths);
        crate::catalog_append::append(log, resolver, run, value, device)?;
    }
    let targets = snapshot(&log.indexes().pending_deletions);
    Ok(Marked { live, targets })
}
