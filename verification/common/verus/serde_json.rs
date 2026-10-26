//! Trusted serde_json primitives; straw event validation remains verified.
//! Keep the audited surface in check_verus.py in sync with this module.
use serde_json::Value;
use std::{borrow::Borrow, hash::Hash};
use vstd::prelude::*;
use vstd::std_specs::btree::{borrowed_key_ordering_matches, borrowed_key_mutated, borrowed_key_removed, contains_borrowed_key, maps_borrowed_key_to_value};

verus! {
    #[verifier::external_trait_specification]
    pub trait ExDeserialize<'a>: Sized {
        type ExternalTraitSpecificationFor: serde::Deserialize<'a>;
    }

    #[verifier::external_trait_specification]
    pub trait ExSerialize {
        type ExternalTraitSpecificationFor: serde::Serialize;
    }

    #[verifier::external_type_specification]
    pub struct ExValue(Value);

    #[verifier::external_type_specification]
    #[verifier::external_body]
    pub struct ExNumber(serde_json::Number);

    #[verifier::external_type_specification]
    #[verifier::external_body]
    pub struct ExError(serde_json::Error);

    #[verifier::external_type_specification]
    #[verifier::external_body]
    #[verifier::reject_recursive_types(K)]
    #[verifier::accept_recursive_types(V)]
    pub struct ExMap<K, V>(serde_json::Map<K, V>);

    #[verifier::external_type_specification]
    #[verifier::external_body]
    pub struct ExIter<'a>(serde_json::map::Iter<'a>);

    pub enum Json {
        Null,
        Boolean(bool),
        Number(Option<u64>),
        Text(Seq<char>),
        Array(Seq<Value>),
        Object(Seq<(String, Value)>),
    }
    pub uninterp spec fn unsigned(number: serde_json::Number) -> Option<u64>;
    pub open spec fn json(value: Value) -> Json {
        match value {
            Value::Null => Json::Null,
            Value::Bool(b) => Json::Boolean(b),
            Value::Number(n) => Json::Number(unsigned(n)),
            Value::String(s) => Json::Text(s@),
            Value::Array(a) => Json::Array(a@),
            Value::Object(m) => Json::Object(entries(m)),
        }
    }
    /// Successful deserialization relation. It is intentionally not a pure
    /// function for arbitrary user-defined Deserialize implementations.
    /// This contract does not assert acceptance or deterministic errors.
    pub uninterp spec fn parses<T>(bytes: Seq<u8>, value: T) -> bool;
    pub uninterp spec fn serializes<T: ?Sized>(value: &T, bytes: Seq<u8>) -> bool;
    // Library law only for serde_json::Value, not arbitrary custom serde types.
    // The relation ignores allocation/layout; parsing acceptance (including
    // depth limits) is not asserted. Cargo enables float_roundtrip.
    #[verifier::external_body]
    pub proof fn value_encoding(value: Value, raw: Seq<u8>)
        requires serializes(&value, raw),
        ensures parses(raw, value),
    {}
    /// Allocation-independent observations used by catalog validation. Numbers
    /// deliberately expose only as_u64; this is not a full numeric-value model.
    pub enum Observation {
        Cut, Null, Boolean(bool), Number(Option<u64>), Text(Seq<char>),
        Array(Seq<Observation>), Object(Seq<(Seq<char>, Observation)>),
    }
    pub open spec fn observe(value: Value, depth: nat) -> Observation
        decreases depth,
    {
        if depth == 0 { Observation::Cut } else {
            match json(value) {
                Json::Null => Observation::Null,
                Json::Boolean(b) => Observation::Boolean(b),
                Json::Number(n) => Observation::Number(n),
                Json::Text(s) => Observation::Text(s),
                Json::Array(items) => Observation::Array(Seq::new(items.len(),
                    |i: int| observe(items[i], (depth - 1) as nat))),
                Json::Object(items) => Observation::Object(Seq::new(items.len(),
                    |i: int| (items[i].0@, observe(items[i].1, (depth - 1) as nat)))),
            }
        }
    }
    // Determinism of Value observations only, not custom Deserialize behavior,
    // allocation identity, acceptance or errors. Object iteration uses the
    // pinned serde_json map configuration; the runner records its features.
    #[verifier::external_body]
    pub proof fn value_parse_consistency(raw: Seq<u8>, left: Value, right: Value, depth: nat)
        requires parses(raw, left), parses(raw, right),
        ensures observe(left, depth) == observe(right, depth),
    {}
    pub uninterp spec fn entries(value: serde_json::Map<String, Value>) -> Seq<(String, Value)>;
    /// The pinned serde_json Map contains at most one entry for each string key.
    /// This is a foreign container representation law, not a straw frame rule.
    #[verifier::external_body]
    pub proof fn map_entries_unique(map: serde_json::Map<String, Value>)
        ensures entries(map).map(|i: int, pair: (String, Value)| pair.0@).no_duplicates(),
    {}
    pub uninterp spec fn remaining<'a>(it: serde_json::map::Iter<'a>) -> Seq<(&'a String, &'a Value)>;
    /// Canonical JSON content, independent of allocation, map layout or Vec capacity.
    /// This is deliberately not Rust/Verus representation equality of Value.
    pub uninterp spec fn content(value: Value) -> Seq<u8>;
    /// Native serde Value equality; distinct numeric representations may be unequal.
    pub uninterp spec fn equal(left: Value, right: Value) -> bool;
    pub assume_specification[ <Value as PartialEq>::eq ](left: &Value, right: &Value) -> (r: bool)
        ensures r == equal(*left, *right),
            r ==> forall|depth: nat| observe(*left, depth) == observe(*right, depth);

    pub open spec fn text(value: Value) -> Option<Seq<char>> {
        match json(value) { Json::Text(t) => Some(t), _ => None }
    }
    pub closed spec fn map_pairs(items: Seq<(String, Value)>) -> Map<String, Value>
        decreases items.len(),
    {
        if items.len() == 0 { Map::empty() }
        else { map_pairs(items.drop_last()).insert(items.last().0, items.last().1) }
    }
    pub proof fn map_pairs_unfold(items: Seq<(String, Value)>)
        ensures map_pairs(items) == if items.len() == 0 { Map::empty() }
            else { map_pairs(items.drop_last()).insert(items.last().0, items.last().1) },
    { reveal(map_pairs); }
    pub open spec fn object(value: serde_json::Map<String, Value>) -> Map<String, Value> {
        map_pairs(entries(value))
    }

    pub open spec fn field_in(items: Seq<(String, Value)>, key: Seq<char>) -> Option<Value>
        decreases items.len(),
    {
        if items.len() == 0 { None }
        else if items[0].0@ == key { Some(items[0].1) }
        else { field_in(items.drop_first(), key) }
    }
    pub open spec fn field(value: Value, key: Seq<char>) -> Option<Value> {
        match json(value) { Json::Object(items) => field_in(items, key), _ => None }
    }

    pub assume_specification[ Value::as_u64 ](value: &Value) -> (r: Option<u64>)
        ensures r == match json(*value) { Json::Number(n) => n, _ => None };
    pub assume_specification<'a>[ <str as serde_json::value::Index>::index_into ](key: &str, value: &'a Value) -> (r: Option<&'a Value>)
        ensures (match r { Some(item) => Some(*item), None => None }) == field(*value, key@);
    pub assume_specification<'a>[ <str as serde_json::value::Index>::index_into_mut ](key: &str, value: &'a mut Value) -> (r: Option<&'a mut Value>)
        ensures match r {
            Some(item) => *old(value) is Object && *final(value) is Object
                && field(*old(value), key@) == Some(*item)
                && field(*final(value), key@) == Some(*final(item))
                && (forall|name: Seq<char>| name != key@ ==> field(*final(value), name) == field(*old(value), name))
                && (*final(item) == *item ==> *final(value) == *old(value)),
            None => field(*old(value), key@).is_none() && *final(value) == *old(value),
        };
    pub assume_specification[ <serde_json::Number as From<u64>>::from ](number: u64) -> (r: serde_json::Number)
        ensures unsigned(r) == Some(number);
    pub assume_specification[ serde_json::Map::<String, Value>::new ]() -> (r: serde_json::Map<String, Value>)
        ensures entries(r) == Seq::<(String, Value)>::empty();
    pub assume_specification[ <Value as Clone>::clone ](value: &Value) -> (r: Value)
        ensures content(r) == content(*value), text(r) == text(*value),
            forall|depth: nat| observe(r, depth) == observe(*value, depth);
    pub assume_specification<'a, Q: ?Sized + Ord + Eq + Hash>[ serde_json::Map::<String, Value>::get::<Q> ](
        value: &'a serde_json::Map<String, Value>, key: &Q) -> (r: Option<&'a Value>)
        where String: Borrow<Q>,
        requires vstd::laws_cmp::obeys_cmp::<String>(), borrowed_key_ordering_matches::<String, Q>(),
        ensures match r {
            Some(v) => maps_borrowed_key_to_value(object(*value), key, *v),
            None => !contains_borrowed_key(object(*value), key),
        };
    pub assume_specification[ serde_json::Map::<String, Value>::insert ](
        value: &mut serde_json::Map<String, Value>, key: String, item: Value) -> (r: Option<Value>)
        requires vstd::laws_cmp::obeys_cmp::<String>(),
        ensures object(*final(value)) == object(*old(value)).insert(key, item),
            forall|name: Seq<char>| field_in(entries(*final(value)), name)
                == if name == key@ { Some(item) } else { field_in(entries(*old(value)), name) },
            r == if object(*old(value)).contains_key(key) { Some(object(*old(value))[key]) } else { None };
    pub assume_specification<'a, Q: ?Sized + Ord + Eq + Hash>[ serde_json::Map::<String, Value>::get_mut::<Q> ](
        map: &'a mut serde_json::Map<String, Value>, key: &Q) -> (r: Option<&'a mut Value>)
        where String: Borrow<Q>,
        requires vstd::laws_cmp::obeys_cmp::<String>(), borrowed_key_ordering_matches::<String, Q>(),
        ensures match r {
            Some(value) => borrowed_key_mutated(object(*old(map)), object(*final(map)), key, *value, *final(value))
                && (*final(value) == *value ==> *final(map) == *old(map)),
            None => !contains_borrowed_key(object(*old(map)), key) && *final(map) == *old(map),
        };
    pub assume_specification<Q: ?Sized + Ord + Eq + Hash>[ serde_json::Map::<String, Value>::remove::<Q> ](
        map: &mut serde_json::Map<String, Value>, key: &Q) -> (r: Option<Value>)
        where String: Borrow<Q>,
        requires vstd::laws_cmp::obeys_cmp::<String>(), borrowed_key_ordering_matches::<String, Q>(),
        ensures borrowed_key_removed(object(*old(map)), object(*final(map)), key),
            match r {
                Some(value) => maps_borrowed_key_to_value(object(*old(map)), key, value),
                None => !contains_borrowed_key(object(*old(map)), key) && *final(map) == *old(map),
            };
    pub assume_specification<F: FnMut(&String, &mut Value) -> bool>[ serde_json::Map::<String, Value>::retain::<F> ](
        map: &mut serde_json::Map<String, Value>, predicate: F)
        requires forall|key: &String, value: &mut Value| call_requires(predicate, (key, value)),
        ensures object(*final(map)).dom().subset_of(object(*old(map)).dom()),
            forall|key: String| object(*old(map)).contains_key(key) ==> exists|value: &mut Value, keep: bool|
                *value == object(*old(map))[key] && call_ensures(predicate, (&key, value), keep)
                    && keep == object(*final(map)).contains_key(key)
                    && (keep ==> object(*final(map))[key] == *final(value));
    pub assume_specification<T: ?Sized + serde::Serialize>[ serde_json::to_vec::<T> ](value: &T) -> (r: Result<Vec<u8>, serde_json::Error>)
        ensures r.is_ok() ==> serializes(value, r->Ok_0@);
    pub assume_specification<'a, T: serde::Deserialize<'a>>[ serde_json::from_slice::<T> ](bytes: &'a [u8]) -> (r: Result<T, serde_json::Error>)
        ensures r.is_ok() ==> parses(bytes@, r->Ok_0);
    pub assume_specification[ Value::as_bool ](value: &Value) -> (r: Option<bool>)
        ensures r == match json(*value) { Json::Boolean(b) => Some(b), _ => None };
    pub assume_specification[ Value::is_null ](value: &Value) -> (r: bool)
        ensures r == (json(*value) == Json::Null);
    pub assume_specification[ Value::as_str ](value: &Value) -> (r: Option<&str>)
        ensures match r {
            Some(s) => json(*value) == Json::Text(s@),
            None => !(json(*value) is Text),
        };
    pub assume_specification[ Value::as_array ](value: &Value) -> (r: Option<&Vec<Value>>)
        ensures match r {
            Some(a) => json(*value) == Json::Array(a@),
            None => !(json(*value) is Array),
        };
    pub assume_specification[ Value::as_object ](value: &Value) -> (r: Option<&serde_json::Map<String, Value>>)
        ensures match r {
            Some(m) => json(*value) == Json::Object(entries(*m)),
            None => !(json(*value) is Object),
        };
    pub assume_specification<'a>[ serde_json::Map::<String, Value>::iter ](value: &'a serde_json::Map<String, Value>) -> (r: serde_json::map::Iter<'a>)
        ensures remaining(r).unref() == entries(*value);
    pub assume_specification<'a>[ <serde_json::map::Iter<'a> as Iterator>::next ](it: &mut serde_json::map::Iter<'a>) -> (r: Option<<serde_json::map::Iter<'a> as Iterator>::Item>)
        ensures match r {
            Some(pair) => remaining(*old(it)).len() > 0 && pair == remaining(*old(it))[0]
                && remaining(*final(it)) == remaining(*old(it)).drop_first(),
            None => remaining(*old(it)).len() == 0 && remaining(*final(it)) == remaining(*old(it)),
        };
}

// Separately audited pinned-serde Value laws; not straw algorithms.
verus! {
    pub open spec fn same_content_depth(original:Value,decoded:Value,depth:nat)->bool
        decreases depth,
    {
        content(original)==content(decoded) && (depth==0 || match json(original) {
            Json::Array(a)=>match json(decoded) {
                Json::Array(b)=>a.len()==b.len() && forall|i:int| 0<=i<a.len()
                    ==> same_content_depth(a[i],b[i],(depth-1) as nat),
                _=>false,
            },
            Json::Object(a)=>match json(decoded) {
                Json::Object(b)=>a.len()==b.len() && forall|i:int| 0<=i<a.len()
                    ==> a[i].0@==b[i].0@ && same_content_depth(a[i].1,b[i].1,(depth-1) as nat),
                _=>false,
            },
            _=>true,
        })
    }
    #[verifier::external_body]
    pub proof fn value_roundtrip_content(original: Value, decoded: Value, raw: Seq<u8>,depth:nat)
        requires serializes(&original, raw), parses(raw, decoded),
        ensures same_content_depth(original,decoded,depth),
    {}
    #[verifier::external_body]
    pub proof fn array_content_congruence(left: Vec<Value>, right: Vec<Value>)
        requires left@.len() == right@.len(),
            forall|i:int| 0 <= i < left@.len() ==> content(left@[i]) == content(right@[i]),
        ensures content(Value::Array(left)) == content(Value::Array(right)),
    {}
    #[verifier::external_body]
    pub proof fn string_content_congruence(left: String, right: String)
        requires left@ == right@,
        ensures content(Value::String(left)) == content(Value::String(right)),
    {}
    #[verifier::external_body]
    pub proof fn unsigned_content_congruence(left: serde_json::Number, right: serde_json::Number)
        requires unsigned(left).is_some(), unsigned(left) == unsigned(right),
        ensures content(Value::Number(left)) == content(Value::Number(right)),
    {}
}
