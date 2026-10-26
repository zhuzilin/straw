//! Actual single record-set validation over normalized manifest/envelope reads.
use crate::closure_node::Digest;
#[cfg(straw_verus)]
use crate::reference_validation::SourceValidity;
use serde_json::{Value, value::Index};
use std::collections::HashSet;
#[cfg(straw_verus)]
use vstd::prelude::*;
#[cfg(straw_verus)]
verus! {
    use crate::json_specs::{json,Json,field};
    pub open spec fn normalized(reference:Value,value:Value)->bool {crate::record_manifest::normalized(reference,value)}
    pub open spec fn envelope(reference:Value,value:Value)->bool {crate::record_index::envelope(reference,value)}
    pub open spec fn item(value:Value,key:Seq<char>)->Value {field(value,key).unwrap_or(Value::Null)}
    pub open spec fn number(value:Value,key:Seq<char>)->u64 {
        match json(item(value,key)) {Json::Number(Some(n))=>n,_=>0}
    }
    pub open spec fn arrays(value:Value,key:Seq<char>)->Option<Seq<Value>> {
        match json(item(value,key)) {Json::Array(items)=>Some(items),_=>None}
    }
    pub open spec fn same_values(before:Seq<Value>,after:Seq<Value>)->bool {
        before.len()==after.len() && forall|i:int| 0<=i<before.len() ==> crate::closure_node::same_reference(before[i],after[i])
    }
    pub open spec fn scope(value:Option<&str>)->Option<Seq<char>> {match value {Some(s)=>Some(s@),None=>None}}
    pub open spec fn scoped(value:Value,task:Option<Seq<char>>,attempt:Option<Seq<char>>)->bool {
        (task.is_some() ==> json(item(item(value,"metadata"@),"task_id"@))==Json::Text(task->Some_0))
            && (attempt.is_some() ==> json(item(item(value,"metadata"@),"attempt_id"@))==Json::Text(attempt->Some_0))
    }
    pub open spec fn digest_keys(values:Seq<Value>,digest:spec_fn(Seq<u8>)->String)->Seq<String> {
        values.map(|i:int,v:Value| digest(crate::json_specs::content(v)))
    }
    pub open spec fn envelopes(refs:Seq<Value>,envs:Seq<Value>,task:Option<Seq<char>>,attempt:Option<Seq<char>>)->bool {
        refs.len()==envs.len() && forall|i:int| 0<=i<refs.len() ==> envelope(refs[i],envs[i]) && scoped(envs[i],task,attempt)
    }
    pub open spec fn sum(values:Seq<Value>,key:Seq<char>)->nat decreases values.len() {
        if values.len()==0 {0} else {sum(values.drop_last(),key)+number(values.last(),key) as nat}
    }
    pub proof fn sum_prefix(values:Seq<Value>,key:Seq<char>,count:nat)
        requires count<=values.len(),ensures sum(values.take(count as int),key)<=sum(values,key),
        decreases values.len()-count,
    {
        if count<values.len() {sum_prefix(values.drop_last(),key,count);assert(values.drop_last().take(count as int)==values.take(count as int));}
        else {assert(values.take(count as int)==values);}
    }
    pub open spec fn candidate(value:Value,envs:Seq<Value>,deps:Seq<Value>)->bool {
        arrays(value,"records"@).is_some() && arrays(value,"dependencies"@).is_some()
            && same_values(envs,arrays(value,"records"@)->Some_0)
            && same_values(deps.map(|i:int,v:Value| item(v,"digest"@)),arrays(value,"dependencies"@)->Some_0)
    }
    pub open spec fn accepted(reference:Value,task:Option<Seq<char>>,attempt:Option<Seq<char>>,records:Seq<Value>,dependencies:Seq<Value>,digest:spec_fn(Seq<u8>)->String)->bool {
        exists|manifest:Value,envs:Seq<Value>,payload:Value| evidence(reference,task,attempt,records,dependencies,digest,manifest,envs,payload)
    }
    pub open spec fn evidence(reference:Value,task:Option<Seq<char>>,attempt:Option<Seq<char>>,records:Seq<Value>,dependencies:Seq<Value>,digest:spec_fn(Seq<u8>)->String,manifest:Value,envs:Seq<Value>,payload:Value)->bool {
            normalized(reference,manifest) && arrays(manifest,"records"@).is_some() && arrays(manifest,"dependencies"@).is_some()
                && arrays(manifest,"records"@)->Some_0.len()==number(reference,"records"@)
                && (forall|name:Seq<char>| name=="records"@ || name=="payload_bytes"@ || name=="tokens"@
                    ==> json(item(reference,name))==Json::Number(Some(number(reference,name))))
                && digest_keys(arrays(manifest,"records"@)->Some_0,digest).no_duplicates()
                && envelopes(arrays(manifest,"records"@)->Some_0,envs,task,attempt)
                && candidate(payload,envs,arrays(manifest,"dependencies"@)->Some_0)
                && json(item(reference,"digest"@))==Json::Text(digest(crate::json_specs::content(payload))@)
                && json(item(manifest,"digest"@))==Json::Text(digest(crate::json_specs::content(payload))@)
                && number(reference,"payload_bytes"@) as nat==sum(envs,"length"@)+sum(arrays(manifest,"dependencies"@)->Some_0,"payload_bytes"@)
                && number(reference,"tokens"@) as nat==sum(envs,"tokens"@)+sum(arrays(manifest,"dependencies"@)->Some_0,"tokens"@)
                && same_values(arrays(manifest,"records"@)->Some_0,records)
                && same_values(arrays(manifest,"dependencies"@)->Some_0,dependencies)
    }
    pub open spec fn members(reference:Value,records:Seq<Value>)->bool {
        exists|task:Option<Seq<char>>,attempt:Option<Seq<char>>,deps:Seq<Value>,digest:spec_fn(Seq<u8>)->String|
            accepted(reference,task,attempt,records,deps,digest)
    }
    pub open spec fn authenticated(key:String,task:Option<Seq<char>>,attempt:Option<Seq<char>>)->bool {
        exists|reference:Value,records:Seq<Value>,deps:Seq<Value>,digest:spec_fn(Seq<u8>)->String|
            key==digest(crate::json_specs::content(reference)) && accepted(reference,task,attempt,records,deps,digest)
    }
    pub open spec fn compatible(g:crate::closure::Graph<Value>,digest:spec_fn(Seq<u8>)->String)->bool {
        crate::closure::valid(g) && (forall|v:Value| (g.key)(v)==digest(crate::json_specs::content(v)))
            && forall|reference:Value,manifest:Value| g.nodes.contains_key((g.key)(reference)) && normalized(reference,manifest) && arrays(manifest,"dependencies"@).is_some()
                ==> crate::reference_validation::dependency_keys(arrays(manifest,"dependencies"@)->Some_0,g.key)==g.nodes[(g.key)(reference)].children
    }
    pub proof fn copied_keys(before:Seq<Value>,after:Seq<Value>,key:spec_fn(Value)->String,digest:spec_fn(Seq<u8>)->String)
        requires same_values(before,after),forall|v:Value| key(v)==digest(crate::json_specs::content(v)),
        ensures crate::reference_validation::dependency_keys(before,key)==crate::reference_validation::dependency_keys(after,key),
    {
        assert(before.map(|i:int,v:Value| key(v)) =~= after.map(|i:int,v:Value| key(v)));
    }
    pub proof fn source_result(g:crate::closure::Graph<Value>,reference:Value,task:Option<Seq<char>>,attempt:Option<Seq<char>>,records:Seq<Value>,deps:Seq<Value>,digest:spec_fn(Seq<u8>)->String)
        requires compatible(g,digest),g.nodes.contains_key((g.key)(reference)),accepted(reference,task,attempt,records,deps,digest),
        ensures members(reference,records),authenticated((g.key)(reference),task,attempt),
            crate::reference_validation::dependency_keys(deps,g.key)==g.nodes[(g.key)(reference)].children,
    {
        let (manifest,envs,payload)=choose|manifest:Value,envs:Seq<Value>,payload:Value| evidence(reference,task,attempt,records,deps,digest,manifest,envs,payload);
        copied_keys(arrays(manifest,"dependencies"@)->Some_0,deps,g.key,digest);
        assert(members(reference,records));assert(authenticated((g.key)(reference),task,attempt));
    }

}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Reader: crate::closure::SourceModel<Reference = Value> {
    type Error;
    #[cfg_attr(straw_verus,verus_spec(result=>requires crate::record_manifest::borrowed_objects(),crate::environment::string_order_contract(),crate::environment::string_content_contract(),ensures final(self)@==old(self)@,result.is_ok() ==> normalized(*reference,result->Ok_0),))]
    fn manifest(&mut self, reference: &Value) -> Result<Value, Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>ensures final(self)@==old(self)@,result.is_ok() ==> envelope(*reference,result->Ok_0),))]
    fn envelope(&mut self, reference: &Value) -> Result<Value, Self::Error>;
}
#[cfg_attr(straw_verus, verus_verify)]
pub enum Error<R, D> {
    Read(R),
    Digest(D),
    Usage,
    Array(&'static str),
    Count,
    Repeated,
    Scope,
    Logical,
    Overflow,
    UsageMismatch,
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures *result==item(*value,key@),))]
fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    match key.index_into(value) {
        Some(v) => v,
        None => &Value::Null,
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures result==number(*value,key@),))]
fn count(value: &Value, key: &str) -> u64 {
    get(value, key).as_u64().unwrap_or(0)
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures result.is_ok()==arrays(*value,key@).is_some(),result.is_ok() ==> result->Ok_0@==arrays(*value,key@)->Some_0,))]
fn array<'a, R, D>(value: &'a Value, key: &'static str) -> Result<&'a Vec<Value>, Error<R, D>> {
    match get(value, key).as_array() {
        Some(a) => Ok(a),
        None => Err(Error::Array(key)),
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures same_values(values@,result@),))]
fn copies(values: &[Value]) -> Vec<Value> {
    let mut output = Vec::new();
    let mut i = 0;
    #[cfg_attr(straw_verus,verus_spec(invariant i<=values.len(),same_values(values@.take(i as int),output@),decreases values.len()-i,))]
    while i < values.len() {
        output.push(values[i].clone());
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(values@.take(i as int)==values@);}
    output
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures same_values(dependencies@.map(|i:int,v:Value| item(v,"digest"@)),result@),))]
fn dependency_digests(dependencies: &[Value]) -> Vec<Value> {
    let mut output = Vec::new();
    let mut i = 0;
    #[cfg_attr(straw_verus,verus_spec(invariant i<=dependencies.len(),output.len()==i,forall|j:int| 0<=j<i ==> crate::closure_node::same_reference(item(dependencies@[j],"digest"@),output@[j]),decreases dependencies.len()-i,))]
    while i < dependencies.len() {
        output.push(get(&dependencies[i], "digest").clone());
        i += 1;
    }
    output
}
#[cfg_attr(straw_verus,verus_spec(result=>requires crate::environment::string_order_contract(),ensures candidate(result,envs@,dependencies@),))]
fn logical_payload(envs: &[Value], dependencies: &[Value]) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("records".to_owned(), Value::Array(copies(envs)));
    object.insert(
        "dependencies".to_owned(),
        Value::Array(dependency_digests(dependencies)),
    );
    Value::Object(object)
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures result.is_some()==(initial as nat+sum(values@,key@)<=u64::MAX),result.is_some() ==> result->Some_0 as nat==initial as nat+sum(values@,key@),))]
fn add_values(values: &[Value], key: &str, initial: u64) -> Option<u64> {
    let mut total = initial;
    let mut i = 0;
    #[cfg_attr(straw_verus,verus_spec(invariant i<=values.len(),total as nat==initial as nat+sum(values@.take(i as int),key@),decreases values.len()-i,))]
    while i < values.len() {
        let value = count(&values[i], key);
        #[cfg(straw_verus)]
        proof! {assert(values@.take(i as int+1).drop_last()==values@.take(i as int));assert(values@.take(i as int+1).last()==values@[i as int]);}
        match crate::bounds::add(total, value) {
            Some(next) => total = next,
            None => {
                #[cfg(straw_verus)]
                proof! {sum_prefix(values@,key@,i as nat+1);}
                return None;
            }
        }
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(values@.take(i as int)==values@);}
    Some(total)
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures result.is_some() ==> result->Some_0.0 as nat==sum(envs@,"length"@)+sum(deps@,"payload_bytes"@) && result->Some_0.1 as nat==sum(envs@,"tokens"@)+sum(deps@,"tokens"@),))]
fn usage(envs: &[Value], deps: &[Value]) -> Option<(u64, u64)> {
    let payload = add_values(envs, "length", 0)?;
    let payload = add_values(deps, "payload_bytes", payload)?;
    let tokens = add_values(envs, "tokens", 0)?;
    let tokens = add_values(deps, "tokens", tokens)?;
    Some((payload, tokens))
}
#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::record_manifest::borrowed_objects(),crate::environment::string_hash_contract(),crate::environment::string_order_contract(),crate::environment::string_content_contract(),
    ensures final(reader)@==old(reader)@,final(digester)@==old(digester)@,result.is_ok() ==> accepted(*reference,scope(task),scope(attempt),result->Ok_0.0@,result->Ok_0.1@,old(digester)@),
))]
// Preserve the checked optional-scope branches and explicit error types.
#[allow(clippy::collapsible_if, clippy::type_complexity)]
pub fn validate<R: Reader, D: Digest>(
    reader: &mut R,
    digester: &mut D,
    reference: &Value,
    task: Option<&str>,
    attempt: Option<&str>,
) -> Result<(Vec<Value>, Vec<Value>), Error<R::Error, D::Error>> {
    if get(reference, "records").as_u64().is_none()
        || get(reference, "payload_bytes").as_u64().is_none()
        || get(reference, "tokens").as_u64().is_none()
    {
        return Err(Error::Usage);
    }
    let manifest = match reader.manifest(reference) {
        Ok(v) => v,
        Err(e) => return Err(Error::Read(e)),
    };
    let refs = array(&manifest, "records")?;
    if refs.len() as u64 != count(reference, "records") {
        return Err(Error::Count);
    }
    let mut known = HashSet::new();
    let mut envs = Vec::new();
    let mut i = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant i<=refs.len(),envs.len()==i,digester@==old(digester)@,reader@==old(reader)@,
            crate::environment::string_hash_contract(),
            known@==digest_keys(refs@.take(i as int),digester@).to_set(),
            digest_keys(refs@.take(i as int),digester@).no_duplicates(),
            envelopes(refs@.take(i as int),envs@,scope(task),scope(attempt)),
        decreases refs.len()-i,
    ))]
    while i < refs.len() {
        let key = match digester.digest(&refs[i]) {
            Ok(key) => key,
            Err(e) => return Err(Error::Digest(e)),
        };
        if !known.insert(key) {
            return Err(Error::Repeated);
        }
        let env = match reader.envelope(&refs[i]) {
            Ok(value) => value,
            Err(e) => return Err(Error::Read(e)),
        };
        if let Some(wanted) = task {
            if get(get(&env, "metadata"), "task_id").as_str() != Some(wanted) {
                return Err(Error::Scope);
            }
        }
        if let Some(wanted) = attempt {
            if get(get(&env, "metadata"), "attempt_id").as_str() != Some(wanted) {
                return Err(Error::Scope);
            }
        }
        #[cfg(straw_verus)]
        proof! {assert(digest_keys(refs@.take(i as int+1),digester@) =~= digest_keys(refs@.take(i as int),digester@).push(key));digest_keys(refs@.take(i as int),digester@).lemma_push_to_set_commute(key);}
        envs.push(env);
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(refs@.take(i as int)==refs@);}
    let deps = array(&manifest, "dependencies")?;
    let payload = logical_payload(&envs, deps);
    let actual = match digester.digest(&payload) {
        Ok(value) => value,
        Err(e) => return Err(Error::Digest(e)),
    };
    if get(reference, "digest").as_str() != Some(actual.as_str())
        || get(&manifest, "digest").as_str() != Some(actual.as_str())
    {
        return Err(Error::Logical);
    }
    let (payload_bytes, tokens) = match usage(&envs, deps) {
        Some(v) => v,
        None => return Err(Error::Overflow),
    };
    if payload_bytes != count(reference, "payload_bytes") || tokens != count(reference, "tokens") {
        return Err(Error::UsageMismatch);
    }
    let records = copies(refs);
    let dependencies = copies(deps);
    #[cfg(straw_verus)]
    proof! {assert(evidence(*reference,scope(task),scope(attempt),records@,dependencies@,digester@,manifest,envs@,payload));assert(accepted(*reference,scope(task),scope(attempt),records@,dependencies@,digester@));}
    Ok((records, dependencies))
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Adapter<'a, R: Reader, D: Digest> {
    reader: &'a mut R,
    digester: D,
}
#[cfg(straw_verus)]
verus! {
    impl<R:Reader,D:Digest> View for Adapter<'_,R,D> {
        type V=crate::closure::Graph<Value>;
        closed spec fn view(&self)->Self::V {self.reader@}
    }
    impl<R:Reader,D:Digest> crate::reference_validation::SourceValidity for Adapter<'_,R,D> {
        closed spec fn valid(&self)->bool {compatible(self.reader@,self.digester@)}
    }
}
#[cfg(not(straw_verus))]
impl<R: Reader, D: Digest> crate::reference_validation::SourceValidity for Adapter<'_, R, D> {}
#[cfg_attr(straw_verus, verus_verify)]
impl<R: Reader, D: Digest> crate::closure::SourceModel for Adapter<'_, R, D> {
    type Reference = Value;
}
#[cfg_attr(straw_verus, verus_verify)]
impl<'a, R: Reader, D: Digest> Adapter<'a, R, D> {
    #[cfg_attr(straw_verus,verus_spec(result=>requires compatible(old(reader)@,digester@),ensures result@==old(reader)@,result.valid(),))]
    pub fn new(reader: &'a mut R, digester: D) -> Self {
        Self { reader, digester }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<R: Reader, D: Digest> crate::reference_validation::Source for Adapter<'_, R, D> {
    type Error = Error<R::Error, D::Error>;
    fn node(
        &mut self,
        reference: &Value,
        task: Option<&str>,
        attempt: Option<&str>,
    ) -> Result<(Vec<Value>, Vec<Value>), Self::Error> {
        let output = validate(self.reader, &mut self.digester, reference, task, attempt);
        #[cfg(straw_verus)]
        proof! {
            if output.is_ok() {source_result(self.reader@,*reference,scope(task),scope(attempt),output->Ok_0.0@,output->Ok_0.1@,self.digester@);}
        }
        output
    }
}

#[cfg(all(test, not(straw_verus)))]
mod tests {
    use super::*;
    use serde_json::json;
    struct Hash;
    impl Digest for Hash {
        type Error = crate::Error;
        fn digest(&mut self, value: &Value) -> crate::Result<String> {
            crate::digest(value)
        }
    }
    struct Input {
        manifest: Value,
        envs: Vec<Value>,
        reads: Vec<Value>,
        manifests: usize,
    }
    impl crate::closure::SourceModel for Input {
        type Reference = Value;
    }
    impl Reader for Input {
        type Error = &'static str;
        fn manifest(&mut self, _: &Value) -> Result<Value, Self::Error> {
            self.manifests += 1;
            Ok(self.manifest.clone())
        }
        fn envelope(&mut self, reference: &Value) -> Result<Value, Self::Error> {
            self.reads.push(reference.clone());
            self.envs
                .get(reference.as_u64().unwrap() as usize)
                .cloned()
                .ok_or("missing envelope")
        }
    }
    fn fixture() -> (Input, Value) {
        let envs = vec![
            json!({"length":7,"tokens":3,"metadata":{"task_id":"task","attempt_id":"attempt"}}),
            json!({"length":11,"tokens":5,"metadata":{"task_id":"task","attempt_id":"attempt"}}),
        ];
        let deps = vec![json!({"digest":"child","payload_bytes":13,"tokens":17})];
        let hash = crate::digest(&json!({"records":envs,"dependencies":["child"]})).unwrap();
        (
            Input {
                manifest: json!({"records":[0,1],"dependencies":deps,"digest":hash}),
                envs,
                reads: vec![],
                manifests: 0,
            },
            json!({"records":2,"payload_bytes":31,"tokens":25,"digest":hash}),
        )
    }
    #[test]
    fn validates_exact_members_scope_digest_and_dependency_accounting() {
        let (mut source, reference) = fixture();
        let (records, deps) = validate(
            &mut source,
            &mut Hash,
            &reference,
            Some("task"),
            Some("attempt"),
        )
        .ok()
        .unwrap();
        assert_eq!(records, vec![json!(0), json!(1)]);
        assert_eq!(
            deps,
            source.manifest["dependencies"].as_array().unwrap().clone()
        );
        assert_eq!(source.reads, vec![json!(0), json!(1)]);
        assert_eq!(source.manifests, 1);
    }
    #[test]
    fn duplicate_and_scope_fail_before_unneeded_reads() {
        let (mut source, reference) = fixture();
        source.manifest["records"] = json!([0, 0]);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::Repeated)
        ));
        assert_eq!(source.reads, vec![json!(0)]);
        let (mut source, reference) = fixture();
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, Some("other"), None),
            Err(Error::Scope)
        ));
        assert_eq!(source.reads, vec![json!(0)]);
    }
    #[test]
    fn malformed_usage_and_count_fail_before_members() {
        let (mut source, mut reference) = fixture();
        reference["tokens"] = json!(-1);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::Usage)
        ));
        assert_eq!(source.manifests, 0);
        reference["tokens"] = json!(25);
        reference["records"] = json!(1);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::Count)
        ));
        assert!(source.reads.is_empty());
    }
    #[test]
    fn verifies_both_digests_before_usage_and_detects_overflow() {
        let (mut source, mut reference) = fixture();
        source.manifest["digest"] = json!("wrong");
        reference["tokens"] = json!(0);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::Logical)
        ));
        let (mut source, mut reference) = fixture();
        reference["digest"] = json!("wrong");
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::Logical)
        ));
        let (mut source, mut reference) = fixture();
        source.envs[0]["length"] = json!(u64::MAX);
        let hash = crate::digest(&json!({"records":source.envs,"dependencies":["child"]})).unwrap();
        source.manifest["digest"] = json!(hash);
        reference["digest"] = json!(hash);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::Overflow)
        ));
    }
    #[test]
    fn compares_both_usage_totals_after_valid_hash() {
        let (mut source, mut reference) = fixture();
        reference["payload_bytes"] = json!(30);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::UsageMismatch)
        ));
        let (mut source, mut reference) = fixture();
        reference["tokens"] = json!(24);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::UsageMismatch)
        ));
    }
}
