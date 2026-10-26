//! Persist exactly the completed deletion targets without reinterpreting paths.
//! The caller separately establishes durable physical absence of these names.
use crate::catalog_decode::Resolver;
use crate::catalog_log::LogState;
use serde_json::Value;

fn event(paths: &[String]) -> Value {
    let mut values = Vec::with_capacity(paths.len());
    let mut i = 0usize;

    while i < paths.len() {
        values.push(Value::String(paths[i].clone()));
        i += 1;
    }

    let mut object = serde_json::Map::new();

    object.insert("reclaimed".to_owned(), Value::Array(values));
    object.insert("path_encoding".to_owned(), Value::Number(1u64.into()));
    Value::Object(object)
}

pub fn record<R: Resolver, D: crate::wal_device::Writer>(
    log: &mut LogState,
    resolver: &mut R,
    run: &str,
    paths: &[String],
    device: &mut D,
) -> Result<(), crate::catalog_append::Error<R::Error, D::Error>> {
    if paths.is_empty() {
        return Ok(());
    }
    let value = event(paths);
    crate::catalog_append::append(log, resolver, run, value, device)?;

    Ok(())
}
