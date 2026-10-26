//! Actual sparse-progress validation, source selection and immutable cache binding.
use serde_json::{Value, value::Index};
use std::{collections::HashSet, sync::Arc};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg(straw_verus)]
verus! {
    use crate::json_specs::{json,Json,observe,field};
    pub open spec fn equivalent(a:Value,b:Value)->bool {
        forall|depth:nat| observe(a,depth)==observe(b,depth)
    }
    pub open spec fn manifest(reference:Value,members:Seq<Value>)->bool {crate::reference_node::members(reference,members)}
    pub uninterp spec fn record(reference:Value,metadata:Value,payload:Seq<u8>)->bool;
    pub open spec fn field_value(value:Value,key:Seq<char>)->Value {field(value,key).unwrap_or(Value::Null)}
    pub open spec fn training_reference(queue:Value)->Value {
        field_value(field_value(field_value(queue,"consumers"@),"training"@),"progress_ref"@)
    }
    pub open spec fn numeric(items:Seq<Value>)->bool {
        forall|i:int| 0<=i<items.len() ==> json(items[i]) is Number && (json(items[i])->Number_0).is_some()
    }
    pub open spec fn textual(items:Seq<Value>)->bool {
        forall|i:int| 0<=i<items.len() ==> json(items[i]) is Text
    }
    pub open spec fn numbers(items:Seq<Value>)->Seq<u64> {
        items.map(|i:int,v:Value| match json(v) {Json::Number(Some(n))=>n,_=>0})
    }
    pub open spec fn texts(items:Seq<Value>)->Seq<Seq<char>> {
        items.map(|i:int,v:Value| match json(v) {Json::Text(s)=>s,_=>Seq::empty()})
    }
    pub open spec fn valid(value:Value)->bool {
        json(field_value(value,"version"@))==Json::Number(Some(1))
            && json(field_value(value,"processed_positions"@)) is Array
            && json(field_value(value,"finished_batches"@)) is Array
            && numeric(json(field_value(value,"processed_positions"@))->Array_0)
            && textual(json(field_value(value,"finished_batches"@))->Array_0)
            && numbers(json(field_value(value,"processed_positions"@))->Array_0).no_duplicates()
            && texts(json(field_value(value,"finished_batches"@))->Array_0).no_duplicates()
    }
    pub open spec fn empty(value:Value)->bool {
        json(field_value(value,"processed_positions"@))==Json::Array(Seq::empty())
            && json(field_value(value,"finished_batches"@))==Json::Array(Seq::empty())
    }
    pub open spec fn authenticated(reference:Value,value:Value)->bool {
        exists|source_ref:Value,members:Seq<Value>,metadata:Value,raw:Seq<u8>,parsed:Value|
            equivalent(reference,source_ref) && manifest(source_ref,members) && members.len()==1
                && record(members[0],metadata,raw) && json(field_value(metadata,"codec"@))==Json::Text("json.v1"@)
                && crate::json_specs::parses(raw,parsed) && valid(parsed) && equivalent(value,parsed)
    }
    pub open spec fn selected(reference:Value,value:Value)->bool {
        if json(reference)==Json::Null {empty(value)} else {authenticated(reference,value)}
    }
    pub proof fn authenticated_copy(reference:Value,value:Value,copy_ref:Value,copy_value:Value)
        requires authenticated(reference,value),equivalent(reference,copy_ref),equivalent(value,copy_value),
        ensures authenticated(copy_ref,copy_value),
    {
        let (source_ref,members,metadata,raw,parsed)=choose|source_ref:Value,members:Seq<Value>,metadata:Value,raw:Seq<u8>,parsed:Value|
            equivalent(reference,source_ref) && manifest(source_ref,members) && members.len()==1
                && record(members[0],metadata,raw) && json(field_value(metadata,"codec"@))==Json::Text("json.v1"@)
                && crate::json_specs::parses(raw,parsed) && valid(parsed) && equivalent(value,parsed);
        assert(equivalent(copy_ref,source_ref)); assert(equivalent(copy_value,parsed));
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Record {
    pub metadata: Value,
    pub payload: Arc<[u8]>,
}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Reader {
    type Error;
    #[cfg_attr(straw_verus,verus_spec(result => ensures result.is_ok() ==> manifest(*reference,result->Ok_0@),))]
    fn members(&mut self, reference: &Value) -> Result<Vec<Value>, Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result => ensures result.is_ok() ==> record(*reference,result->Ok_0.metadata,result->Ok_0.payload@),))]
    fn read(&mut self, reference: &Value) -> Result<Record, Self::Error>;
}
#[cfg_attr(straw_verus, verus_verify)]
pub enum Error<E> {
    Read(E),
    Members,
    Codec,
    Json(serde_json::Error),
    Array(&'static str),
    Invalid,
    Duplicate,
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct Cache {
    entry: Option<(Value, Value)>,
}
#[cfg(straw_verus)]
verus! {impl Cache {
    pub closed spec fn valid(&self)->bool {match self.entry {None=>true,Some((reference,value))=>authenticated(reference,value)}}
}}
#[cfg_attr(straw_verus, verus_verify)]
impl Cache {
    #[cfg_attr(straw_verus,verus_spec(result=>ensures result.valid(),))]
    pub fn new() -> Self {
        Self { entry: None }
    }
}

#[cfg_attr(straw_verus,verus_spec(result=>ensures *result==field_value(*value,key@),))]
fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    match key.index_into(value) {
        Some(v) => v,
        None => &Value::Null,
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures result.is_ok() ==> json(*value)==crate::json_specs::Json::Array(result->Ok_0@),))]
fn array<'a, E>(value: &'a Value, name: &'static str) -> Result<&'a Vec<Value>, Error<E>> {
    match value.as_array() {
        Some(a) => Ok(a),
        None => Err(Error::Array(name)),
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures result==numeric(items@),))]
fn numeric_values(items: &[Value]) -> bool {
    let mut i = 0;
    #[cfg_attr(straw_verus,verus_spec(invariant i<=items.len(),forall|j:int| 0<=j<i ==> crate::json_specs::json(items@[j]) is Number && (crate::json_specs::json(items@[j])->Number_0).is_some(), decreases items.len()-i,))]
    while i < items.len() {
        if items[i].as_u64().is_none() {
            return false;
        }
        i += 1;
    }
    true
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures result==textual(items@),))]
fn text_values(items: &[Value]) -> bool {
    let mut i = 0;
    #[cfg_attr(straw_verus,verus_spec(invariant i<=items.len(),forall|j:int| 0<=j<i ==> crate::json_specs::json(items@[j]) is Text, decreases items.len()-i,))]
    while i < items.len() {
        if items[i].as_str().is_none() {
            return false;
        }
        i += 1;
    }
    true
}
#[cfg_attr(straw_verus,verus_spec(result=>requires numeric(items@),ensures result==numbers(items@).no_duplicates(),))]
fn unique_numbers(items: &[Value]) -> bool {
    let mut known = HashSet::new();
    let mut i = 0;
    #[cfg_attr(straw_verus,verus_spec(invariant i<=items.len(),numeric(items@),known@==numbers(items@.take(i as int)).to_set(),numbers(items@.take(i as int)).no_duplicates(),decreases items.len()-i,))]
    while i < items.len() {
        let n = items[i].as_u64().unwrap();
        if !known.insert(n) {
            #[cfg(straw_verus)]
            proof! {let j=choose|j:int| 0<=j<i && numbers(items@.take(i as int))[j]==n; assert(numbers(items@)[j]==numbers(items@)[i as int]);}
            return false;
        }
        #[cfg(straw_verus)]
        proof! {assert(numbers(items@.take(i as int+1)) =~= numbers(items@.take(i as int)).push(n)); numbers(items@.take(i as int)).lemma_push_to_set_commute(n);}
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(items@.take(i as int)==items@);}
    true
}
#[cfg_attr(straw_verus,verus_spec(result=>requires textual(items@),crate::environment::string_hash_contract(),crate::environment::string_content_contract(),ensures result==texts(items@).no_duplicates(),))]
fn unique_texts(items: &[Value]) -> bool {
    let mut known: HashSet<String> = HashSet::new();
    let mut i = 0;
    #[cfg_attr(straw_verus,verus_spec(invariant i<=items.len(),textual(items@),crate::environment::string_hash_contract(),crate::environment::string_content_contract(),
        (forall|name:String| known@.contains(name) ==> texts(items@.take(i as int)).to_set().contains(name@)),
        (forall|j:int| 0<=j<i ==> exists|name:String| known@.contains(name) && name@==#[trigger] texts(items@)[j]),
        texts(items@.take(i as int)).no_duplicates(),decreases items.len()-i,))]
    while i < items.len() {
        let name = items[i].as_str().unwrap().to_owned();
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(known@));
        if !known.insert(name) {
            #[cfg(straw_verus)]
            proof! {let j=choose|j:int| 0<=j<i && texts(items@.take(i as int))[j]==name@;assert(texts(items@)[j]==texts(items@)[i as int]);}
            return false;
        }
        #[cfg(straw_verus)]
        proof! {
            assert(texts(items@.take(i as int+1)) =~= texts(items@.take(i as int)).push(name@));
            texts(items@.take(i as int)).lemma_push_to_set_commute(name@);
            assert forall|j:int| 0<=j<i implies texts(items@)[j]!=name@ by {
                let old_name=choose|old_name:String| before@.contains(old_name) && old_name@==#[trigger] texts(items@)[j];
                if old_name@==name@ {assert(old_name==name);}
            }
            if texts(items@.take(i as int)).contains(name@) {
                let j=choose|j:int| 0<=j<i && texts(items@.take(i as int))[j]==name@;
                assert(texts(items@.take(i as int))[j]==texts(items@)[j]);
            }
            assert(!texts(items@.take(i as int)).contains(name@));
            assert(texts(items@.take(i as int)).push(name@).no_duplicates());
        }
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(items@.take(i as int)==items@);}
    true
}
#[cfg_attr(straw_verus,verus_spec(result=>requires crate::environment::string_hash_contract(),crate::environment::string_content_contract(),ensures result.is_ok() ==> valid(*value),))]
fn validate<E>(value: &Value) -> Result<(), Error<E>> {
    if get(value, "version").as_u64() != Some(1) {
        return Err(Error::Invalid);
    }
    let positions = array(get(value, "processed_positions"), "processed_positions")?;
    if !numeric_values(positions) {
        return Err(Error::Invalid);
    }
    let batches = array(get(value, "finished_batches"), "finished_batches")?;
    if !text_values(batches) {
        return Err(Error::Invalid);
    }
    if !unique_numbers(positions) || !unique_texts(batches) {
        return Err(Error::Duplicate);
    }
    Ok(())
}
#[cfg_attr(straw_verus,verus_spec(result=>requires crate::environment::string_order_contract(),ensures empty(result),))]
fn empty_value() -> Value {
    let mut object = serde_json::Map::new();
    object.insert("processed_positions".to_owned(), Value::Array(Vec::new()));
    object.insert("finished_batches".to_owned(), Value::Array(Vec::new()));
    Value::Object(object)
}
#[cfg_attr(straw_verus,verus_spec(result=>requires old(cache).valid(),crate::environment::string_order_contract(),crate::environment::string_hash_contract(),crate::environment::string_content_contract(),
    ensures final(cache).valid(),result.is_ok() ==> selected(*reference,result->Ok_0),
))]
#[allow(
    clippy::collapsible_if,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn load<R: Reader>(
    cache: &mut Cache,
    reader: &mut R,
    reference: &Value,
) -> Result<Value, Error<R::Error>> {
    if reference.is_null() {
        return Ok(empty_value());
    }
    if let Some((cached_ref, cached_value)) = &cache.entry {
        if *cached_ref == *reference {
            let result = cached_value.clone();
            #[cfg(straw_verus)]
            proof! {authenticated_copy(*cached_ref,*cached_value,*reference,result);}
            return Ok(result);
        }
    }
    let members = match reader.members(reference) {
        Ok(m) => m,
        Err(e) => return Err(Error::Read(e)),
    };
    if members.len() != 1 {
        return Err(Error::Members);
    }
    let data = match reader.read(&members[0]) {
        Ok(data) => data,
        Err(e) => return Err(Error::Read(e)),
    };
    if get(&data.metadata, "codec").as_str() != Some("json.v1") {
        return Err(Error::Codec);
    }
    let parsed: Value = match serde_json::from_slice(&data.payload) {
        Ok(p) => p,
        Err(e) => return Err(Error::Json(e)),
    };
    validate(&parsed)?;
    #[cfg(straw_verus)]
    proof! {assert(equivalent(*reference,*reference));assert(equivalent(parsed,parsed));assert(authenticated(*reference,parsed));}
    let cached_ref = reference.clone();
    let cached_value = parsed.clone();
    #[cfg(straw_verus)]
    proof! {authenticated_copy(*reference,parsed,cached_ref,cached_value);}
    cache.entry = Some((cached_ref, cached_value));
    Ok(parsed)
}
#[cfg_attr(straw_verus,verus_spec(result=>requires old(cache).valid(),crate::environment::string_order_contract(),crate::environment::string_hash_contract(),crate::environment::string_content_contract(),
    ensures final(cache).valid(),result.is_ok() ==> selected(training_reference(*queue),result->Ok_0),
))]
pub fn training<R: Reader>(
    cache: &mut Cache,
    reader: &mut R,
    queue: &Value,
) -> Result<Value, Error<R::Error>> {
    load(
        cache,
        reader,
        get(get(get(queue, "consumers"), "training"), "progress_ref"),
    )
}

#[cfg(all(test, not(straw_verus)))]
mod tests {
    use super::*;
    use serde_json::json;
    #[derive(Default)]
    struct Storage {
        calls: Vec<String>,
        value: Value,
        count: usize,
        codec: String,
    }
    impl Storage {
        fn valid(value: Value) -> Self {
            Self {
                value,
                count: 1,
                codec: "json.v1".into(),
                ..Self::default()
            }
        }
    }
    impl Reader for Storage {
        type Error = ();
        fn members(&mut self, reference: &Value) -> Result<Vec<Value>, ()> {
            self.calls.push(format!("members:{reference}"));
            Ok((0..self.count)
                .map(|i| json!({"root":reference,"ordinal":i}))
                .collect())
        }
        fn read(&mut self, reference: &Value) -> Result<Record, ()> {
            self.calls.push(format!("record:{reference}"));
            Ok(Record {
                metadata: json!({"codec":self.codec}),
                payload: serde_json::to_vec(&self.value).unwrap().into(),
            })
        }
    }
    fn progress(positions: Value, batches: Value) -> Value {
        json!({"version":1,"processed_positions":positions,"finished_batches":batches})
    }
    #[test]
    fn cache_tracks_the_selected_reference_and_rejects_failed_replacement() {
        let first = progress(json!([2, 9]), json!(["a"]));
        let second = progress(json!([10]), json!(["b"]));
        let mut source = Storage::valid(first.clone());
        let mut cache = Cache::new();
        let a = json!({"digest":"a"});
        let b = json!({"digest":"b"});
        assert_eq!(load(&mut cache, &mut source, &a).ok(), Some(first.clone()));
        assert_eq!(source.calls.len(), 2);
        source.value = Value::Null;
        assert_eq!(
            load(&mut cache, &mut source, &a.clone()).ok(),
            Some(first.clone())
        );
        assert_eq!(source.calls.len(), 2);
        assert!(matches!(
            load(&mut cache, &mut source, &b),
            Err(Error::Invalid)
        ));
        assert_eq!(source.calls.len(), 4);
        assert_eq!(load(&mut cache, &mut source, &a).ok(), Some(first));
        assert_eq!(source.calls.len(), 4);
        source.value = second.clone();
        assert_eq!(
            training(
                &mut cache,
                &mut source,
                &json!({"consumers":{"training":{"progress_ref":b}}})
            )
            .ok(),
            Some(second)
        );
        assert_eq!(source.calls.len(), 6);
    }
    #[test]
    fn null_progress_bypasses_cached_nonempty_state_and_storage() {
        let mut source = Storage::valid(progress(json!([7]), json!(["finished"])));
        let mut cache = Cache::new();
        assert!(load(&mut cache, &mut source, &json!({"digest":"old"})).is_ok());
        source.calls.clear();
        assert_eq!(
            load(&mut cache, &mut source, &Value::Null).ok(),
            Some(json!({"processed_positions":[],"finished_batches":[]}))
        );
        assert!(source.calls.is_empty());
    }
    #[test]
    fn progress_validation_preserves_type_duplicate_and_error_priority() {
        for (value, expected) in [
            (progress(json!([1, 1]), json!([])), "duplicate"),
            (progress(json!([]), json!(["二", "二"])), "duplicate"),
            (progress(json!([1.0]), json!([])), "invalid"),
            (progress(json!([]), json!([false])), "invalid"),
            (json!({"version":1,"processed_positions":[1,1]}), "array"),
            (json!({"version":2}), "invalid"),
        ] {
            let mut source = Storage::valid(value);
            let mut cache = Cache::new();
            let actual = match load(&mut cache, &mut source, &json!({"digest":"test"})) {
                Err(Error::Duplicate) => "duplicate",
                Err(Error::Invalid) => "invalid",
                Err(Error::Array(_)) => "array",
                _ => "unexpected",
            };
            assert_eq!(actual, expected);
            assert!(cache.entry.is_none());
        }
    }
    #[test]
    fn wrong_record_count_and_codec_are_rejected_before_parsing() {
        let mut source = Storage::valid(Value::Null);
        source.count = 2;
        let mut cache = Cache::new();
        assert!(matches!(
            load(&mut cache, &mut source, &json!({"digest":"test"})),
            Err(Error::Members)
        ));
        assert_eq!(source.calls.len(), 1);
        source.count = 1;
        source.codec = "bytes.v1".into();
        assert!(matches!(
            load(&mut cache, &mut source, &json!({"digest":"test"})),
            Err(Error::Codec)
        ));
        assert!(cache.entry.is_none());
    }
}
