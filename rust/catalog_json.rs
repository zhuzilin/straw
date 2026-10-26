//! Catalog wire types. Parsing and type checks run before path resolution.
#[cfg(straw_verus)]
use crate::json_specs::{Json, entries, field, field_in, json, remaining};
use serde_json::Value;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub struct Invalid;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum DecodeError {
    Object,
    Encoding,
    Owners,
    Packs,
    Deleted,
    Reclaimed,
    Edges,
    Receipts,
}

pub type OwnerList = Vec<(String, Option<Vec<String>>)>;

#[cfg_attr(straw_verus, verus_verify)]
pub struct Wire {
    pub encoding: Option<u64>,
    pub owners: OwnerList,
    pub packs: Vec<(String, bool)>,
    pub deleted: Vec<String>,
    pub reclaimed: Vec<String>,
    pub edges: OwnerList,
    pub receipts: Vec<String>,
    pub has_receipts: bool,
    pub has_owners: bool,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures (match result { Some(v) => Some(*v), None => None }) == field(*value, key@),
))]
pub fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let object = value.as_object()?;
    let mut it = object.iter();
    #[cfg_attr(straw_verus, verus_spec(
        invariant field_in(remaining(it).unref(), key@) == field(*value, key@),
        decreases remaining(it).len(),
    ))]
    loop {
        #[cfg(straw_verus)]
        proof! {
            reveal(field_in);
            assert(remaining(it).unref().len() == remaining(it).len());
            if remaining(it).len() > 0 {
                assert(remaining(it).unref()[0] == (*remaining(it)[0].0, *remaining(it)[0].1));
                assert(remaining(it).drop_first().unref() =~= remaining(it).unref().drop_first());
            }
        }
        let (name, item) = it.next()?;
        if name.as_str() == key {
            return Some(item);
        }
    }
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn string_array(value: Value) -> bool {
        json(value) is Array && forall|i: int| 0 <= i < (json(value)->Array_0).len()
            ==> json((json(value)->Array_0)[i]) is Text
    }
    pub open spec fn strings_match(value: Value, result: Seq<String>) -> bool {
        json(value) is Array && result.len() == (json(value)->Array_0).len()
            && forall|i: int| 0 <= i < result.len()
                ==> json((json(value)->Array_0)[i]) == Json::Text(result[i]@)
    }
    pub open spec fn owner_value(value: Value, allow_null: bool) -> bool {
        (allow_null && json(value) == Json::Null) || string_array(value)
    }
    pub open spec fn owner_match(value: Value, result: Option<Vec<String>>, allow_null: bool) -> bool {
        match result {
            None => allow_null && json(value) == Json::Null,
            Some(paths) => strings_match(value, paths@),
        }
    }
    pub open spec fn owner_map(value: Value, allow_null: bool) -> bool {
        json(value) is Object && forall|i: int| 0 <= i < (json(value)->Object_0).len()
            ==> owner_value((json(value)->Object_0)[i].1, allow_null)
    }
    pub open spec fn owners_match(value: Value, result: Seq<(String, Option<Vec<String>>)>, allow_null: bool) -> bool {
        json(value) is Object && result.len() == (json(value)->Object_0).len()
            && forall|i: int| 0 <= i < result.len() ==> result[i].0 == (json(value)->Object_0)[i].0
                && owner_match((json(value)->Object_0)[i].1, result[i].1, allow_null)
    }
    pub open spec fn pack_map(value: Value) -> bool {
        json(value) is Object && forall|i: int| 0 <= i < (json(value)->Object_0).len()
            ==> json((json(value)->Object_0)[i].1) is Boolean
    }
    pub open spec fn packs_match(value: Value, result: Seq<(String, bool)>) -> bool {
        json(value) is Object && result.len() == (json(value)->Object_0).len()
            && forall|i: int| 0 <= i < result.len() ==> result[i].0 == (json(value)->Object_0)[i].0
                && json((json(value)->Object_0)[i].1) == Json::Boolean(result[i].1)
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == string_array(*value),
        result.is_ok() ==> strings_match(*value, result.unwrap()@),
))]
pub fn strings(value: &Value) -> Result<Vec<String>, Invalid> {
    let Some(items) = value.as_array() else {
        return Err(Invalid);
    };
    let mut result: Vec<String> = Vec::new();
    let mut i = 0;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= items.len(), result.len() == i, json(*value) == Json::Array(items@),
            forall|j: int| 0 <= j < i ==> json(items@[j]) == Json::Text(result@[j]@),
        decreases items.len() - i,
    ))]
    while i < items.len() {
        let Some(text) = items[i].as_str() else {
            #[cfg(straw_verus)]
            proof! { assert(!string_array(*value)); }
            return Err(Invalid);
        };
        result.push(text.to_owned());
        i += 1;
    }
    Ok(result)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == owner_map(*value, allow_null),
        result.is_ok() ==> owners_match(*value, result.unwrap()@, allow_null),
))]
pub fn owners(value: &Value, allow_null: bool) -> Result<OwnerList, Invalid> {
    let Some(object) = value.as_object() else {
        return Err(Invalid);
    };
    let mut it = object.iter();
    let mut result: Vec<(String, Option<Vec<String>>)> = Vec::new();
    #[cfg_attr(straw_verus, verus_spec(
        invariant_except_break json(*value) == Json::Object(entries(*object)),
            result.len() <= entries(*object).len(),
            remaining(it).unref() == entries(*object).subrange(result.len() as int, entries(*object).len() as int),
            forall|i: int| 0 <= i < result.len() ==> result@[i].0 == entries(*object)[i].0
                && owner_match(entries(*object)[i].1, result@[i].1, allow_null)
                && owner_value(entries(*object)[i].1, allow_null),
        decreases remaining(it).len(),
    ))]
    loop {
        #[cfg(straw_verus)]
        proof! {
            assert(remaining(it).unref().len() == remaining(it).len());
            if remaining(it).len() > 0 {
                assert(remaining(it).unref()[0] == (*remaining(it)[0].0, *remaining(it)[0].1));
                assert(remaining(it).drop_first().unref() =~= remaining(it).unref().drop_first());
            }
        }
        let Some((key, item)) = it.next() else {
            #[cfg(straw_verus)]
            proof! { assert(result.len() == entries(*object).len()); }
            return Ok(result);
        };
        #[cfg(straw_verus)]
        proof! { assert(result.len() < entries(*object).len()); assert((*key, *item) == entries(*object)[result.len() as int]); }
        let paths = if allow_null && item.is_null() {
            None
        } else {
            match strings(item) {
                Ok(paths) => Some(paths),
                Err(error) => {
                    #[cfg(straw_verus)]
                    proof! { assert(!owner_value(entries(*object)[result.len() as int].1, allow_null)); if owner_map(*value, allow_null) {
                        assert(owner_value((json(*value)->Object_0)[result.len() as int].1, allow_null));
                    } }
                    return Err(error);
                }
            }
        };
        result.push((key.clone(), paths));
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == pack_map(*value),
        result.is_ok() ==> packs_match(*value, result.unwrap()@),
))]
pub fn packs(value: &Value) -> Result<Vec<(String, bool)>, Invalid> {
    let Some(object) = value.as_object() else {
        return Err(Invalid);
    };
    let mut it = object.iter();
    let mut result: Vec<(String, bool)> = Vec::new();
    #[cfg_attr(straw_verus, verus_spec(
        invariant_except_break json(*value) == Json::Object(entries(*object)),
            result.len() <= entries(*object).len(),
            remaining(it).unref() == entries(*object).subrange(result.len() as int, entries(*object).len() as int),
            forall|i: int| 0 <= i < result.len() ==> result@[i].0 == entries(*object)[i].0
                && json(entries(*object)[i].1) == Json::Boolean(result@[i].1),
        decreases remaining(it).len(),
    ))]
    loop {
        #[cfg(straw_verus)]
        proof! {
            assert(remaining(it).unref().len() == remaining(it).len());
            if remaining(it).len() > 0 {
                assert(remaining(it).unref()[0] == (*remaining(it)[0].0, *remaining(it)[0].1));
                assert(remaining(it).drop_first().unref() =~= remaining(it).unref().drop_first());
            }
        }
        let Some((key, item)) = it.next() else {
            #[cfg(straw_verus)]
            proof! { assert(result.len() == entries(*object).len()); }
            return Ok(result);
        };
        #[cfg(straw_verus)]
        proof! { assert(result.len() < entries(*object).len()); assert((*key, *item) == entries(*object)[result.len() as int]); }
        let Some(sealed) = item.as_bool() else {
            #[cfg(straw_verus)]
            proof! { assert(!(json(entries(*object)[result.len() as int].1) is Boolean)); }
            return Err(Invalid);
        };
        result.push((key.clone(), sealed));
    }
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn optional_strings(value: Option<Value>, result: Seq<String>) -> bool {
        match value { None => result.len() == 0, Some(v) => strings_match(v, result) }
    }
    pub open spec fn optional_owners(value: Option<Value>, result: Seq<(String, Option<Vec<String>>)>, allow_null: bool) -> bool {
        match value { None => result.len() == 0, Some(v) => owners_match(v, result, allow_null) }
    }
    pub open spec fn optional_packs(value: Option<Value>, result: Seq<(String, bool)>) -> bool {
        match value { None => result.len() == 0, Some(v) => packs_match(v, result) }
    }
    pub open spec fn optional_number(value: Option<Value>, result: Option<u64>) -> bool {
        match value { None => result == None, Some(v) => json(v) == Json::Number(result) && result.is_some() }
    }
    pub open spec fn schema(value: Value) -> bool {
        json(value) is Object
            && match field(value, "path_encoding"@) { None => true, Some(v) => json(v) is Number && (json(v)->Number_0).is_some() }
            && match field(value, "owners"@) { None => true, Some(v) => owner_map(v, true) }
            && match field(value, "packs"@) { None => true, Some(v) => pack_map(v) }
            && match field(value, "deleted"@) { None => true, Some(v) => string_array(v) }
            && match field(value, "reclaimed"@) { None => true, Some(v) => string_array(v) }
            && match field(value, "staging_edges"@) { None => true, Some(v) => owner_map(v, false) }
            && match field(value, "consumed_handoffs"@) { None => true, Some(v) => string_array(v) }
    }
    pub open spec fn wire_matches(value: Value, result: Wire) -> bool {
        optional_number(field(value, "path_encoding"@), result.encoding)
            && optional_owners(field(value, "owners"@), result.owners@, true)
            && optional_packs(field(value, "packs"@), result.packs@)
            && optional_strings(field(value, "deleted"@), result.deleted@)
            && optional_strings(field(value, "reclaimed"@), result.reclaimed@)
            && optional_owners(field(value, "staging_edges"@), result.edges@, false)
            && optional_strings(field(value, "consumed_handoffs"@), result.receipts@)
            && result.has_receipts == field(value, "consumed_handoffs"@).is_some()
            && result.has_owners == field(value, "owners"@).is_some()
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == schema(*value), result.is_ok() ==> wire_matches(*value, result.unwrap()),
))]
pub fn decode(value: &Value) -> Result<Wire, DecodeError> {
    if value.as_object().is_none() {
        return Err(DecodeError::Object);
    }
    let has_receipts = get(value, "consumed_handoffs").is_some();
    let encoding = match get(value, "path_encoding") {
        None => None,
        Some(v) => match v.as_u64() {
            Some(version) => Some(version),
            None => return Err(DecodeError::Encoding),
        },
    };
    let owners = match get(value, "owners") {
        None => Vec::new(),
        Some(v) => match owners(v, true) {
            Ok(items) => items,
            Err(_) => {
                return Err(if has_receipts {
                    DecodeError::Receipts
                } else {
                    DecodeError::Owners
                });
            }
        },
    };
    let packs = match get(value, "packs") {
        None => Vec::new(),
        Some(v) => match packs(v) {
            Ok(items) => items,
            Err(_) => return Err(DecodeError::Packs),
        },
    };
    let deleted = match get(value, "deleted") {
        None => Vec::new(),
        Some(v) => match strings(v) {
            Ok(items) => items,
            Err(_) => return Err(DecodeError::Deleted),
        },
    };
    let reclaimed = match get(value, "reclaimed") {
        None => Vec::new(),
        Some(v) => match strings(v) {
            Ok(items) => items,
            Err(_) => return Err(DecodeError::Reclaimed),
        },
    };
    let edges = match get(value, "staging_edges") {
        None => Vec::new(),
        Some(v) => match self::owners(v, false) {
            Ok(items) => items,
            Err(_) => return Err(DecodeError::Edges),
        },
    };
    let receipts = match get(value, "consumed_handoffs") {
        None => Vec::new(),
        Some(v) => match strings(v) {
            Ok(items) => items,
            Err(_) => return Err(DecodeError::Receipts),
        },
    };
    Ok(Wire {
        encoding,
        owners,
        packs,
        deleted,
        reclaimed,
        edges,
        receipts,
        has_receipts,
        has_owners: get(value, "owners").is_some(),
    })
}
