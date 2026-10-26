//! Catalog wire types. Parsing and type checks run before path resolution.

use serde_json::Value;

#[derive(Debug)]
pub struct Invalid;

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

pub fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let object = value.as_object()?;
    let mut it = object.iter();

    loop {
        let (name, item) = it.next()?;
        if name.as_str() == key {
            return Some(item);
        }
    }
}

pub fn strings(value: &Value) -> Result<Vec<String>, Invalid> {
    let Some(items) = value.as_array() else {
        return Err(Invalid);
    };
    let mut result: Vec<String> = Vec::new();
    let mut i = 0;

    while i < items.len() {
        let Some(text) = items[i].as_str() else {
            return Err(Invalid);
        };
        result.push(text.to_owned());
        i += 1;
    }
    Ok(result)
}

pub fn owners(value: &Value, allow_null: bool) -> Result<OwnerList, Invalid> {
    let Some(object) = value.as_object() else {
        return Err(Invalid);
    };
    let mut it = object.iter();
    let mut result: Vec<(String, Option<Vec<String>>)> = Vec::new();

    loop {
        let Some((key, item)) = it.next() else {
            return Ok(result);
        };

        let paths = if allow_null && item.is_null() {
            None
        } else {
            match strings(item) {
                Ok(paths) => Some(paths),
                Err(error) => {
                    return Err(error);
                }
            }
        };
        result.push((key.clone(), paths));
    }
}

pub fn packs(value: &Value) -> Result<Vec<(String, bool)>, Invalid> {
    let Some(object) = value.as_object() else {
        return Err(Invalid);
    };
    let mut it = object.iter();
    let mut result: Vec<(String, bool)> = Vec::new();

    loop {
        let Some((key, item)) = it.next() else {
            return Ok(result);
        };

        let Some(sealed) = item.as_bool() else {
            return Err(Invalid);
        };
        result.push((key.clone(), sealed));
    }
}

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
