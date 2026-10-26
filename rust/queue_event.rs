//! Actual task-ID selection before queue replay updates scheduling indexes.
use serde_json::Value;
use serde_json::value::Index;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error {
    StringField(&'static str),
    ArrayField(&'static str),
    Identity,
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Prepared<'a> {
    pub kind: &'a str,
    pub affected: Vec<String>,
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn selector(key: Option<&str>) -> Option<Seq<char>> {
        match key { Some(name) => Some(name@), None => None }
    }
    pub open spec fn text(value: Value, key: Seq<char>) -> Option<Seq<char>> {
        crate::json_specs::text(crate::request::field_or_null(value, key))
    }
    pub open spec fn identity(value: Value, key: Option<Seq<char>>) -> Option<Seq<char>> {
        match key { Some(name) => text(value, name), None => crate::json_specs::text(value) }
    }
    pub open spec fn names_valid(items: Seq<Value>, key: Option<Seq<char>>) -> bool {
        forall|i: int| 0 <= i < items.len() ==> identity(items[i], key).is_some()
    }
    pub open spec fn names(items: Seq<Value>, key: Option<Seq<char>>) -> Seq<Seq<char>> {
        Seq::new(items.len(), |i: int| identity(items[i], key)->Some_0)
    }
    pub open spec fn array_targets(event: Value, field: Seq<char>, key: Option<Seq<char>>) -> Option<Seq<Seq<char>>> {
        match crate::json_specs::json(crate::request::field_or_null(event, field)) {
            crate::json_specs::Json::Array(items) => if names_valid(items, key) { Some(names(items, key)) } else { None },
            _ => None,
        }
    }
    pub open spec fn single_target(event: Value, key: Seq<char>) -> Option<Seq<Seq<char>>> {
        match text(event, key) { Some(id) => Some(seq![id]), None => None }
    }
    pub open spec fn targets(event: Value) -> Option<Seq<Seq<char>>> {
        match text(event, "type"@) {
            None => None,
            Some(kind) => if kind == "Submitted"@ { array_targets(event, "tasks"@, Some("task_id"@)) }
                else if kind == "Leased"@ { array_targets(event, "leases"@, Some("task_id"@)) }
                else if kind == "Epoch"@ { array_targets(event, "revoked"@, None) }
                else if kind == "Expired"@ || kind == "Failed"@ { array_targets(event, "task_ids"@, None) }
                else if kind == "Cancelled"@ || kind == "Yielded"@ || kind == "TaskProgress"@ { single_target(event, "task_id"@) }
                else if kind == "Completed"@ { single_target(crate::request::field_or_null(event, "receipt"@), "task_id"@) }
                else { Some(Seq::empty()) },
        }
    }
    pub open spec fn prepared(event: Value, result: Prepared) -> bool {
        Some(result.kind@) == text(event, "type"@) && targets(event).is_some()
            && result.affected@.len() == targets(event)->Some_0.len()
            && forall|i: int| 0 <= i < result.affected.len()
                ==> result.affected@[i]@ == targets(event)->Some_0[i]
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == text(*value, name@).is_some(),
        result.is_ok() ==> result->Ok_0@ == text(*value, name@)->Some_0,
))]
fn string_field<'a>(value: &'a Value, name: &'static str) -> Result<&'a str, Error> {
    match name.index_into(value).and_then(Value::as_str) {
        Some(value) => Ok(value),
        None => Err(Error::StringField(name)),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == (crate::json_specs::json(crate::request::field_or_null(*value, name@)) is Array),
        result.is_ok() ==> crate::json_specs::json(crate::request::field_or_null(*value, name@))
            == crate::json_specs::Json::Array(result->Ok_0@),
))]
fn array_field<'a>(value: &'a Value, name: &'static str) -> Result<&'a Vec<Value>, Error> {
    match name.index_into(value).and_then(Value::as_array) {
        Some(value) => Ok(value),
        None => Err(Error::ArrayField(name)),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == names_valid(items@, selector(key)),
        result.is_ok() ==> result->Ok_0@.len() == items@.len()
            && forall|i: int| 0 <= i < items.len() ==> result->Ok_0@[i]@ == names(items@, selector(key))[i],
))]
fn collect(items: &[Value], key: Option<&'static str>) -> Result<Vec<String>, Error> {
    let mut result: Vec<String> = Vec::with_capacity(items.len());
    let mut index = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant index <= items.len(), result.len() == index,
            forall|i: int| 0 <= i < index ==> identity(items@[i], selector(key)).is_some(),
            forall|i: int| 0 <= i < index ==> result@[i]@ == identity(items@[i], selector(key))->Some_0,
        decreases items.len() - index,
    ))]
    while index < items.len() {
        let id = match key {
            Some(name) => string_field(&items[index], name)?,
            None => match items[index].as_str() {
                Some(id) => id,
                None => return Err(Error::Identity),
            },
        };
        #[cfg(straw_verus)]
        proof! { assert(identity(items@[index as int], selector(key)) == Some(id@)); }
        result.push(id.to_owned());
        index += 1;
    }
    Ok(result)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == targets(*event).is_some(),
        result.is_ok() ==> prepared(*event, result->Ok_0),
))]
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
