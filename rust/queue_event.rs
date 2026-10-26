//! Actual task-ID selection before queue replay updates scheduling indexes.
use serde_json::Value;
use serde_json::value::Index;

#[derive(Debug)]
pub enum Error {
    StringField(&'static str),
    ArrayField(&'static str),
    Identity,
}

pub struct Prepared<'a> {
    pub kind: &'a str,
    pub affected: Vec<String>,
}

fn string_field<'a>(value: &'a Value, name: &'static str) -> Result<&'a str, Error> {
    match name.index_into(value).and_then(Value::as_str) {
        Some(value) => Ok(value),
        None => Err(Error::StringField(name)),
    }
}

fn array_field<'a>(value: &'a Value, name: &'static str) -> Result<&'a Vec<Value>, Error> {
    match name.index_into(value).and_then(Value::as_array) {
        Some(value) => Ok(value),
        None => Err(Error::ArrayField(name)),
    }
}

fn collect(items: &[Value], key: Option<&'static str>) -> Result<Vec<String>, Error> {
    let mut result: Vec<String> = Vec::with_capacity(items.len());
    let mut index = 0usize;

    while index < items.len() {
        let id = match key {
            Some(name) => string_field(&items[index], name)?,
            None => match items[index].as_str() {
                Some(id) => id,
                None => return Err(Error::Identity),
            },
        };

        result.push(id.to_owned());
        index += 1;
    }
    Ok(result)
}

pub fn prepare(event: &Value) -> Result<Prepared<'_>, Error> {
    let kind = string_field(event, "type")?;
    let affected = if kind == "Submitted" {
        collect(array_field(event, "tasks")?, Some("task_id"))?
    } else if kind == "Leased" {
        collect(array_field(event, "leases")?, Some("task_id"))?
    } else if kind == "Epoch" {
        collect(array_field(event, "revoked")?, None)?
    } else if kind == "Expired" || kind == "Failed" {
        collect(array_field(event, "task_ids")?, None)?
    } else if kind == "Cancelled" || kind == "Yielded" || kind == "TaskProgress" {
        vec![string_field(event, "task_id")?.to_owned()]
    } else if kind == "Completed" {
        let Some(receipt) = "receipt".index_into(event) else {
            return Err(Error::StringField("task_id"));
        };
        vec![string_field(receipt, "task_id")?.to_owned()]
    } else {
        Vec::new()
    };
    Ok(Prepared { kind, affected })
}
