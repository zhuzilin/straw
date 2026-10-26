//! Actual v1/v2 record-set decoding; v2 local ordinals become scoped references.
use serde_json::{Value, value::Index};
#[cfg(straw_verus)]
use vstd::prelude::*;
#[cfg(straw_verus)]
verus! {
    use crate::json_specs::{json,Json,field};
    pub open spec fn get(value:Value,key:Seq<char>)->Value {field(value,key).unwrap_or(Value::Null)}
    pub open spec fn count(value:Value,key:Seq<char>)->u64 {match json(get(value,key)) {Json::Number(Some(n))=>n,_=>0}}
    pub open spec fn local(parent:Value,ordinal:Value,reference:Value)->bool {
        json(ordinal) is Number && (json(ordinal)->Number_0).is_some()
            && (json(ordinal)->Number_0)->Some_0<count(get(parent,"manifest"@),"ordinal"@)
            && crate::closure_node::same_reference(get(get(parent,"manifest"@),"segment"@),get(reference,"segment"@))
            && json(get(reference,"ordinal"@))==json(ordinal)
            && forall|key:Seq<char>| key!="segment"@ && key!="ordinal"@ ==> field(reference,key).is_none()
    }
    pub open spec fn dependency(parent:Value,before:Value,after:Value)->bool {
        if field(before,"local"@).is_none() {after==before}
        else {field(after,"local"@).is_none() && local(parent,get(before,"local"@),get(after,"manifest"@))
            && forall|key:Seq<char>| key!="local"@ && key!="manifest"@ ==> field(before,key)==field(after,key)}
    }
    pub open spec fn converted(parent:Value,before:Value,after:Value)->bool {
        json(get(before,"records"@)) is Array && json(get(after,"records"@)) is Array
            && (json(get(before,"records"@))->Array_0).len()==(json(get(after,"records"@))->Array_0).len()
            && (forall|i:int| 0<=i<(json(get(before,"records"@))->Array_0).len()
                ==> local(parent,(json(get(before,"records"@))->Array_0)[i],(json(get(after,"records"@))->Array_0)[i]))
            && json(get(before,"dependencies"@)) is Array && json(get(after,"dependencies"@)) is Array
            && (json(get(before,"dependencies"@))->Array_0).len()==(json(get(after,"dependencies"@))->Array_0).len()
            && (forall|i:int| 0<=i<(json(get(before,"dependencies"@))->Array_0).len()
                ==> dependency(parent,(json(get(before,"dependencies"@))->Array_0)[i],(json(get(after,"dependencies"@))->Array_0)[i]))
            && forall|key:Seq<char>| key!="records"@ && key!="dependencies"@ ==> field(before,key)==field(after,key)
    }
    pub open spec fn decoded(parent:Value,metadata:Value,parsed:Value,value:Value)->bool {
        (json(get(metadata,"codec"@))==Json::Text("record-set.v1"@) && count(parsed,"version"@)==1 && value==parsed)
            || (json(get(metadata,"codec"@))==Json::Text("record-set.v2"@) && count(parsed,"version"@)==2 && converted(parent,parsed,value))
    }
    pub open spec fn removed(before:Map<String,Value>,after:Map<String,Value>,name:Seq<char>)->bool {
        (forall|k:String| after.contains_key(k)==(before.contains_key(k) && k@!=name))
            && (forall|k:String| after.contains_key(k) ==> after[k]==before[k])
    }
    // Standard-library String/str borrowed-key correspondence; no conversion behavior is assumed.
    pub open spec fn borrowed_objects()->bool {
        vstd::std_specs::btree::borrowed_key_ordering_matches::<String,str>()
            && forall|before:Map<String,Value>,after:Map<String,Value>,name:&str|
                vstd::std_specs::btree::borrowed_key_removed(before,after,name)==removed(before,after,name@)
    }
    pub proof fn removal(before:serde_json::Map<String,Value>,after:serde_json::Map<String,Value>,key:Seq<char>)
        requires crate::environment::string_content_contract(),removed(crate::json_specs::object(before),crate::json_specs::object(after),key),
        ensures forall|name:Seq<char>| field(Value::Object(after),name)==if name==key {None} else {field(Value::Object(before),name)},
    {
        crate::json_specs::map_entries_unique(before);crate::json_specs::map_entries_unique(after);
        crate::queue_gc::unique_from_map(crate::json_specs::entries(before));crate::queue_gc::unique_from_map(crate::json_specs::entries(after));
        assert forall|name:Seq<char>| field(Value::Object(after),name)==if name==key {None} else {field(Value::Object(before),name)} by {
            crate::queue_gc::map_field(crate::json_specs::entries(before),name);crate::queue_gc::map_field(crate::json_specs::entries(after),name);
            if field(Value::Object(before),name).is_some() {
                let a=choose|a:String| crate::json_specs::object(before).contains_key(a) && a@==name && crate::json_specs::object(before)[a]==field(Value::Object(before),name)->Some_0;
                if field(Value::Object(after),name).is_some() {
                    let b=choose|b:String| crate::json_specs::object(after).contains_key(b) && b@==name && crate::json_specs::object(after)[b]==field(Value::Object(after),name)->Some_0;assert(a==b);
                }
            } else if field(Value::Object(after),name).is_some() {
                let b=choose|b:String| crate::json_specs::object(after).contains_key(b) && b@==name && crate::json_specs::object(after)[b]==field(Value::Object(after),name)->Some_0;
            }
        }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
pub enum Error {
    Json(serde_json::Error),
    Schema,
    Records,
    Dependencies,
    Ordinal,
    Precedence,
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures *result==get(*value,key@),))]
fn item<'a>(value: &'a Value, key: &str) -> &'a Value {
    match key.index_into(value) {
        Some(v) => v,
        None => &Value::Null,
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures result==count(*value,key@),))]
fn number(value: &Value, key: &str) -> u64 {
    item(value, key).as_u64().unwrap_or(0)
}
#[cfg_attr(straw_verus,verus_spec(result=>requires crate::environment::string_order_contract(),ensures result.is_ok() ==> local(*parent,*ordinal,result->Ok_0),))]
fn local_ref(parent: &Value, ordinal: &Value) -> Result<Value, Error> {
    let n = match ordinal.as_u64() {
        Some(n) => n,
        None => return Err(Error::Ordinal),
    };
    if n >= number(item(parent, "manifest"), "ordinal") {
        return Err(Error::Precedence);
    }
    let mut map = serde_json::Map::new();
    map.insert(
        "segment".to_owned(),
        item(item(parent, "manifest"), "segment").clone(),
    );
    map.insert("ordinal".to_owned(), Value::Number(n.into()));
    Ok(Value::Object(map))
}
#[cfg_attr(straw_verus,verus_spec(result=>requires borrowed_objects(),crate::environment::string_order_contract(),crate::environment::string_content_contract(),ensures result.is_ok() ==> dependency(*parent,*old(value),*final(value)),))]
fn convert_dependency(parent: &Value, value: &mut Value) -> Result<(), Error> {
    #[cfg(straw_verus)]
    proof! {broadcast use vstd::std_specs::btree::group_btree_axioms;}
    #[cfg(straw_verus)]
    let original = verus_exec_expr!(Ghost(*value));
    let Some(ordinal) = "local".index_into(value) else {
        return Ok(());
    };
    let reference = local_ref(parent, ordinal)?;
    let Value::Object(map) = value else {
        return Ok(());
    };
    #[cfg(straw_verus)]
    let before = verus_exec_expr!(Ghost(*map));
    map.remove("local");
    #[cfg(straw_verus)]
    proof! {
        assert(removed(crate::json_specs::object(before@),crate::json_specs::object(*map),"local"@));
        removal(before@,*map,"local"@);
        assert(field(Value::Object(*map),"local"@).is_none());
        assert forall|key:Seq<char>| key!="local"@ implies field(original@,key)==field(Value::Object(*map),key) by {};

    }
    #[cfg(straw_verus)]
    let middle = verus_exec_expr!(Ghost(*map));
    map.insert("manifest".to_owned(), reference);
    #[cfg(straw_verus)]
    proof! {
        assert(field(Value::Object(*map),"local"@)==field(Value::Object(middle@),"local"@));
        assert(field(Value::Object(*map),"local"@).is_none());
        assert(local(*parent,get(original@,"local"@),get(Value::Object(*map),"manifest"@)));
        assert forall|key:Seq<char>| key!="local"@ && key!="manifest"@ implies field(original@,key)==field(Value::Object(*map),key) by {};
        assert(dependency(*parent,original@,Value::Object(*map)));
    }
    Ok(())
}
#[cfg_attr(straw_verus,verus_spec(result=>requires borrowed_objects(),crate::environment::string_order_contract(),crate::environment::string_content_contract(),ensures result.is_ok() ==> decoded(*parent,*metadata,value,result->Ok_0),))]
pub fn normalize(parent: &Value, metadata: &Value, mut value: Value) -> Result<Value, Error> {
    let codec = item(metadata, "codec").as_str();
    let version = number(&value, "version");
    if codec == Some("record-set.v1") && version == 1 {
        return Ok(value);
    }
    if codec != Some("record-set.v2") || version != 2 {
        return Err(Error::Schema);
    }
    let Some(records) = item(&value, "records").as_array() else {
        return Err(Error::Records);
    };
    let mut output = Vec::new();
    let mut i = 0;
    #[cfg_attr(straw_verus,verus_spec(invariant i<=records.len(),output.len()==i,crate::environment::string_order_contract(),forall|j:int| 0<=j<i ==> local(*parent,records@[j],output@[j]),decreases records.len()-i,))]
    while i < records.len() {
        output.push(local_ref(parent, &records[i])?);
        i += 1;
    }
    let Value::Object(map) = &mut value else {
        return Err(Error::Records);
    };
    map.insert("records".to_owned(), Value::Array(output));
    let Some(deps) = "dependencies".index_into_mut(&mut value) else {
        return Err(Error::Dependencies);
    };
    let Value::Array(deps) = deps else {
        return Err(Error::Dependencies);
    };
    #[cfg(straw_verus)]
    let initial = verus_exec_expr!(Ghost(deps@));
    let mut i = 0;
    #[cfg_attr(straw_verus,verus_spec(invariant i<=deps.len(),deps.len()==initial@.len(),borrowed_objects(),crate::environment::string_order_contract(),crate::environment::string_content_contract(),forall|j:int| 0<=j<i ==> dependency(*parent,initial@[j],deps@[j]),forall|j:int| i<=j<deps.len() ==> deps@[j]==initial@[j],decreases deps.len()-i,))]
    while i < deps.len() {
        convert_dependency(parent, &mut deps[i])?;
        i += 1;
    }
    Ok(value)
}
#[cfg_attr(straw_verus,verus_spec(result=>requires borrowed_objects(),crate::environment::string_order_contract(),crate::environment::string_content_contract(),ensures result.is_ok() ==> exists|parsed:Value| crate::json_specs::parses(raw@,parsed) && decoded(*parent,*metadata,parsed,result->Ok_0),))]
pub fn decode(parent: &Value, metadata: &Value, raw: &[u8]) -> Result<Value, Error> {
    let value = match serde_json::from_slice::<Value>(raw) {
        Ok(v) => v,
        Err(e) => return Err(Error::Json(e)),
    };
    normalize(parent, metadata, value)
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn normalized(reference:Value,value:Value)->bool {
        exists|metadata:Value,raw:Seq<u8>,parsed:Value|
            crate::queue_progress::record(get(reference,"manifest"@),metadata,raw)
                && crate::json_specs::parses(raw,parsed) && decoded(reference,metadata,parsed,value)
    }
}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Reader: crate::closure::SourceModel<Reference = Value> {
    type Error;
    #[cfg_attr(straw_verus,verus_spec(result=>ensures final(self)@==old(self)@,result.is_ok() ==> crate::queue_progress::record(*reference,result->Ok_0.metadata,result->Ok_0.payload@),))]
    fn record(&mut self, reference: &Value) -> Result<crate::queue_progress::Record, Self::Error>;
}
#[cfg_attr(straw_verus, verus_verify)]
pub enum ReadError<E> {
    Read(E),
    Decode(Error),
}
#[cfg_attr(straw_verus,verus_spec(result=>requires borrowed_objects(),crate::environment::string_order_contract(),crate::environment::string_content_contract(),ensures final(reader)@==old(reader)@,result.is_ok() ==> normalized(*reference,result->Ok_0),))]
pub fn read<R: Reader>(reader: &mut R, reference: &Value) -> Result<Value, ReadError<R::Error>> {
    let record = match reader.record(item(reference, "manifest")) {
        Ok(r) => r,
        Err(e) => return Err(ReadError::Read(e)),
    };
    let value = match decode(reference, &record.metadata, &record.payload) {
        Ok(v) => v,
        Err(e) => return Err(ReadError::Decode(e)),
    };
    Ok(value)
}

#[cfg(all(test, not(straw_verus)))]
mod tests {
    use super::*;
    use serde_json::json;
    fn metadata(version: u64) -> Value {
        json!({"codec":format!("record-set.v{version}")})
    }
    #[test]
    fn v1_passes_through_and_schema_pair_is_exact() {
        let reference = json!({});
        let original = json!({"version":1,"opaque":-0.0});
        assert_eq!(
            normalize(&reference, &metadata(1), original.clone()).ok(),
            Some(original)
        );
        assert!(matches!(
            normalize(&reference, &metadata(2), json!({"version":1})),
            Err(Error::Schema)
        ));
        assert!(matches!(
            normalize(&reference, &metadata(1), json!({"version":2})),
            Err(Error::Schema)
        ));
    }
    #[test]
    fn v2_expands_local_ordinals_preserves_external_dependencies_and_fields() {
        let reference = json!({"manifest":{"segment":{"run":"run","id":"pack"},"ordinal":8}});
        let original = json!({"version":2,"records":[0,7],"dependencies":[{"local":3,"digest":"a","other":9},{"manifest":{"foreign":true},"digest":"b"}],"digest":"result","other":[1,2]});
        let value = normalize(&reference, &metadata(2), original).ok().unwrap();
        assert_eq!(
            value,
            json!({"version":2,"records":[{"segment":{"run":"run","id":"pack"},"ordinal":0},{"segment":{"run":"run","id":"pack"},"ordinal":7}],"dependencies":[{"manifest":{"segment":{"run":"run","id":"pack"},"ordinal":3},"digest":"a","other":9},{"manifest":{"foreign":true},"digest":"b"}],"digest":"result","other":[1,2]})
        );
    }
    #[test]
    fn local_references_require_unsigned_preceding_ordinals() {
        let reference = json!({"manifest":{"segment":"pack","ordinal":8}});
        for ordinal in [json!(-1), json!(1.0), json!("1"), Value::Null] {
            assert!(matches!(
                normalize(
                    &reference,
                    &metadata(2),
                    json!({"version":2,"records":[ordinal],"dependencies":[]})
                ),
                Err(Error::Ordinal)
            ));
        }
        for ordinal in [8, 9, u64::MAX] {
            assert!(matches!(
                normalize(
                    &reference,
                    &metadata(2),
                    json!({"version":2,"records":[ordinal],"dependencies":[]})
                ),
                Err(Error::Precedence)
            ));
            assert!(matches!(
                normalize(
                    &reference,
                    &metadata(2),
                    json!({"version":2,"records":[],"dependencies":[{"local":ordinal}]})
                ),
                Err(Error::Precedence)
            ));
        }
    }
    #[test]
    fn malformed_arrays_and_json_keep_error_precedence() {
        let reference = json!({"manifest":{"ordinal":8}});
        assert!(matches!(
            decode(&reference, &metadata(2), b"{"),
            Err(Error::Json(_))
        ));
        assert!(matches!(
            normalize(
                &reference,
                &metadata(2),
                json!({"version":2,"dependencies":[]})
            ),
            Err(Error::Records)
        ));
        assert!(matches!(
            normalize(&reference, &metadata(2), json!({"version":2,"records":[]})),
            Err(Error::Dependencies)
        ));
        assert!(matches!(
            normalize(
                &reference,
                &metadata(2),
                json!({"version":2,"records":[9],"dependencies":"bad"})
            ),
            Err(Error::Precedence)
        ));
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Envelopes: crate::closure::SourceModel<Reference = Value> {
    type Error;
    #[cfg_attr(straw_verus,verus_spec(result=>ensures final(self)@==old(self)@,result.is_ok() ==> crate::reference_node::envelope(*reference,result->Ok_0),))]
    fn envelope(&mut self, reference: &Value) -> Result<Value, Self::Error>;
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct Adapter<'a, R: Reader + Envelopes<Error = <R as Reader>::Error>> {
    source: &'a mut R,
}
#[cfg(straw_verus)]
verus! {impl<R:Reader+Envelopes<Error=<R as Reader>::Error>> View for Adapter<'_,R> {type V=crate::closure::Graph<Value>;closed spec fn view(&self)->Self::V {self.source@}}}
#[cfg_attr(straw_verus, verus_verify)]
impl<R: Reader + Envelopes<Error = <R as Reader>::Error>> crate::closure::SourceModel
    for Adapter<'_, R>
{
    type Reference = Value;
}
#[cfg_attr(straw_verus, verus_verify)]
impl<'a, R: Reader + Envelopes<Error = <R as Reader>::Error>> Adapter<'a, R> {
    #[cfg_attr(straw_verus,verus_spec(result=>ensures result@==old(source)@,))]
    pub fn new(source: &'a mut R) -> Self {
        Self { source }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<R: Reader + Envelopes<Error = <R as Reader>::Error>> crate::reference_node::Reader
    for Adapter<'_, R>
{
    type Error = ReadError<<R as Reader>::Error>;
    fn manifest(&mut self, reference: &Value) -> Result<Value, Self::Error> {
        read(self.source, reference)
    }
    fn envelope(&mut self, reference: &Value) -> Result<Value, Self::Error> {
        match self.source.envelope(reference) {
            Ok(v) => Ok(v),
            Err(e) => Err(ReadError::Read(e)),
        }
    }
}
