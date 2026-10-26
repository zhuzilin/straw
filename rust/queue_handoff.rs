//! Production handoff envelope and exact raw roundtrip correspondence.
use crate::closure_node::Digest;
use serde_json::Value;
use std::collections::HashMap;
#[cfg(straw_verus)]
use vstd::prelude::*;
#[cfg(straw_verus)]
verus! {
    pub open spec fn marked(value:Value)->bool {
        crate::json_specs::text(crate::queue_roots::field_or_null(value,"type"@))==Some("StagingHandoff"@)
    }
    pub open spec fn unmarked(events:Seq<Value>)->bool {
        forall|i:int| 0<=i<events.len() ==> !marked(events[i])
    }
    pub open spec fn identity(id:String,prefix:Seq<char>,sequence:u64,business:Seq<Value>,digest:spec_fn(Seq<u8>)->String)->bool {
        (exists|value:Value| package(value,prefix,sequence,business))
            && forall|value:Value| package(value,prefix,sequence,business) ==> id==digest(crate::json_specs::content(value))
    }
    pub open spec fn values_cover(values:Seq<Value>,roots:Map<String,Value>)->bool {
        values.to_set()==roots.values()
    }
    pub open spec fn recovered(result:(String,Vec<Value>),events:Seq<Value>,prefix:Seq<char>,sequence:u64,digest:spec_fn(Seq<u8>)->String)->bool {
        events.len()>0 && marked(events.last()) && unmarked(events.drop_last())
            && crate::json_specs::text(crate::queue_roots::field_or_null(events.last(),"id"@))==Some(result.0@)
            && identity(result.0,prefix,sequence,events.drop_last(),digest)
            && exists|roots:Map<String,Value>| values_cover(result.1@,roots)
                && roots.dom()!=Set::<String>::empty()
                && crate::queue_roots::valid_keys(roots,digest)
                && roots.dom().to_iset()==crate::queue_roots::many_keys(events.drop_last(),digest)
    }
    pub open spec fn shaped(events:Seq<Value>)->bool {
        events.len()>0 && marked(events.last()) && unmarked(events.drop_last())
    }
    pub open spec fn recovery_result<E>(result:Result<Option<(String,Vec<Value>)>,Error<E>>,transaction:Value,prefix:Seq<char>,sequence:u64,digest:spec_fn(Seq<u8>)->String)->bool {
        match result {
            Err(Error::Transaction)=> !(transaction is Array),
            _=> match crate::json_specs::json(transaction) {
                crate::json_specs::Json::Array(events)=> match result {
                    Ok(None)=>unmarked(events),
                    Ok(Some(value))=>recovered(value,events,prefix,sequence,digest),
                    Err(Error::Misplaced)=>events.len()>0 && !marked(events.last()) && !unmarked(events),
                    Err(Error::Repeated)=>events.len()>0 && marked(events.last()) && !unmarked(events.drop_last()),
                    Err(Error::Identity)=>shaped(events) && exists|id:String| identity(id,prefix,sequence,events.drop_last(),digest)
                        && crate::json_specs::text(crate::queue_roots::field_or_null(events.last(),"id"@))!=Some(id@),
                    Err(Error::Empty)=>shaped(events) && crate::queue_roots::many_keys(events.drop_last(),digest)==ISet::<String>::empty(),
                    Err(Error::Digest(_))=>shaped(events),
                    _=>false,
                },
                _=>false,
            },
        }
    }
    pub open spec fn contents(items:Seq<Value>)->Seq<Seq<u8>> {
        items.map(|i:int,v:Value| crate::json_specs::content(v))
    }
    pub open spec fn package(value:Value,prefix:Seq<char>,sequence:u64,business:Seq<Value>)->bool {
        match crate::json_specs::json(value) {
            crate::json_specs::Json::Array(parts)=>parts.len()==3
                && crate::json_specs::json(parts[0])==crate::json_specs::Json::Text(prefix)
                && crate::json_specs::json(parts[1])==crate::json_specs::Json::Number(Some(sequence))
                && match crate::json_specs::json(parts[2]) {
                    crate::json_specs::Json::Array(items)=>contents(items)==contents(business),
                    _=>false,
                },
            _=>false,
        }
    }
    pub proof fn package_consistency(left:Value,right:Value,prefix:Seq<char>,sequence:u64,a:Seq<Value>,b:Seq<Value>)
        requires package(left,prefix,sequence,a),package(right,prefix,sequence,b),contents(a)==contents(b),
        ensures crate::json_specs::content(left)==crate::json_specs::content(right),
    {
        let x=match left {Value::Array(value)=>value,_=>arbitrary()};
        let y=match right {Value::Array(value)=>value,_=>arbitrary()};
        let xs=match x@[0] {Value::String(value)=>value,_=>arbitrary()};
        let ys=match y@[0] {Value::String(value)=>value,_=>arbitrary()};
        crate::json_specs::string_content_congruence(xs,ys);
        let xn=match x@[1] {Value::Number(value)=>value,_=>arbitrary()};
        let yn=match y@[1] {Value::Number(value)=>value,_=>arbitrary()};
        crate::json_specs::unsigned_content_congruence(xn,yn);
        let xa=match x@[2] {Value::Array(value)=>value,_=>arbitrary()};
        let ya=match y@[2] {Value::Array(value)=>value,_=>arbitrary()};
        assert(contents(xa@)==contents(a));
        assert(contents(ya@)==contents(b));
        assert(contents(xa@)==contents(ya@));
        assert(contents(xa@).len()==xa@.len());
        assert(contents(ya@).len()==ya@.len());
        assert(xa@.len()==ya@.len());
        assert forall|i:int| 0<=i<xa@.len() implies crate::json_specs::content(xa@[i])==crate::json_specs::content(ya@[i]) by {
            assert(contents(xa@)[i]==contents(ya@)[i]);
        }
        crate::json_specs::array_content_congruence(xa,ya);
        assert forall|i:int| 0<=i<3 implies crate::json_specs::content(x@[i])==crate::json_specs::content(y@[i]) by {
            if i==0 {} else if i==1 {} else {assert(i==2);}
        }
        crate::json_specs::array_content_congruence(x,y);
    }
    pub proof fn recovered_preimage(original:Value,decoded:Value,raw:Seq<u8>,count:int,
        first:Value,rebuilt:Value,prefix:Seq<char>,sequence:u64)
        requires crate::json_specs::serializes(&original,raw),crate::json_specs::parses(raw,decoded),
            original is Array, 0<=count<=crate::json_specs::json(original)->Array_0.len(),
            decoded is Array,
            package(first,prefix,sequence,crate::json_specs::json(original)->Array_0.take(count)),
            package(rebuilt,prefix,sequence,crate::json_specs::json(decoded)->Array_0.take(count)),
        ensures crate::json_specs::content(first)==crate::json_specs::content(rebuilt),
    {
        crate::json_specs::value_roundtrip_content(original,decoded,raw,1);
        reveal_with_fuel(crate::json_specs::same_content_depth,2);
        let a=crate::json_specs::json(original)->Array_0.take(count);let b=crate::json_specs::json(decoded)->Array_0.take(count);
        assert(a.len()==b.len());
        assert forall|i:int| 0<=i<a.len() implies contents(a)[i]==contents(b)[i] by {
            assert(crate::json_specs::same_content_depth(a[i],b[i],0));
        }
        assert(contents(a) =~= contents(b));
        package_consistency(first,rebuilt,prefix,sequence,a,b);
    }
    pub proof fn identity_roundtrip(original:Value,decoded:Value,raw:Seq<u8>,
        prefix:Seq<char>,sequence:u64,first:String,rebuilt:String,digest:spec_fn(Seq<u8>)->String)
        requires crate::json_specs::serializes(&original,raw),crate::json_specs::parses(raw,decoded),
            original is Array,decoded is Array,crate::json_specs::json(original)->Array_0.len()>0,
            identity(first,prefix,sequence,crate::json_specs::json(original)->Array_0.drop_last(),digest),
            identity(rebuilt,prefix,sequence,crate::json_specs::json(decoded)->Array_0.drop_last(),digest),
        ensures first==rebuilt,
    {
        crate::json_specs::value_roundtrip_content(original,decoded,raw,1);
        reveal_with_fuel(crate::json_specs::same_content_depth,2);
        let a=crate::json_specs::json(original)->Array_0;let b=crate::json_specs::json(decoded)->Array_0;
        assert(a.len()==b.len());
        let left=choose|v:Value| package(v,prefix,sequence,a.drop_last());
        let right=choose|v:Value| package(v,prefix,sequence,b.drop_last());
        assert(a.take(a.len()-1)==a.drop_last());assert(b.take(a.len()-1)==b.drop_last());
        recovered_preimage(original,decoded,raw,a.len()-1,left,right,prefix,sequence);
        assert(first==digest(crate::json_specs::content(left)));
        assert(rebuilt==digest(crate::json_specs::content(right)));
    }
    /// Use the raw payload selected by the actual WAL frame parser, rather than
    /// inferring canonical content from Value equality or schema observations.
    pub(crate) proof fn accepted_replayed_identity(before:crate::journal_state::State,after:crate::journal_state::State,
        original:Value,bytes:Seq<u8>,raw:Seq<Seq<u8>>,transactions:Seq<Value>,
        prefix:Seq<char>,first:String,rebuilt:String,digest:spec_fn(Seq<u8>)->String)
        requires crate::journal_append::accepted(before,after,original,bytes),
            crate::journal_recovery::decoded(raw,transactions),
            crate::wal_recovery::replayed(crate::framing::Kind::Queue,bytes,0,0,Seq::empty(),raw),
            before.sequence<raw.len(),
            before.offset==crate::wal_recovery::run_end(0,raw.take(before.sequence as int)),
            original is Array,crate::json_specs::json(original)->Array_0.len()>0,
            identity(first,prefix,before.sequence,crate::json_specs::json(original)->Array_0.drop_last(),digest),
            identity(rebuilt,prefix,before.sequence,crate::json_specs::json(transactions[before.sequence as int])->Array_0.drop_last(),digest),
        ensures first==rebuilt,
    {
        let payload=choose|payload:Seq<u8>| crate::journal_append::encoded(original,payload)
            && crate::wal_recovery::complete_frame(crate::framing::Kind::Queue,bytes,before.offset as int,before.sequence as nat,payload,after.offset as int);
        assert(raw.subrange(0,raw.len() as int)==raw);
        crate::wal_recovery::run_record(crate::framing::Kind::Queue,bytes,0,0,raw,before.sequence as nat);
        crate::wal_recovery::frame_unique(crate::framing::Kind::Queue,bytes,before.offset as int,before.sequence as nat,
            payload,after.offset as int,raw[before.sequence as int],crate::wal_recovery::run_end(0,raw.take(before.sequence as int+1)));
        identity_roundtrip(original,transactions[before.sequence as int],payload,prefix,before.sequence,first,rebuilt,digest);
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures package(result,prefix@,sequence,business@)))]
#[allow(
    clippy::vec_init_then_push,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn envelope(prefix: &str, sequence: u64, business: &[Value]) -> Value {
    let mut items = Vec::with_capacity(business.len());
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=business.len(),items.len()==index,
            forall|i:int| 0<=i<index ==> crate::json_specs::content(items@[i])==crate::json_specs::content(business@[i]),
        decreases business.len()-index,
    ))]
    while index < business.len() {
        items.push(business[index].clone());
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(contents(items@) =~= contents(business@));}
    let mut parts = Vec::new();
    parts.push(Value::String(prefix.to_owned()));
    parts.push(Value::Number(sequence.into()));
    parts.push(Value::Array(items));
    Value::Array(parts)
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error<E> {
    Transaction,
    Misplaced,
    Repeated,
    Identity,
    Empty,
    Digest(E),
}

#[cfg_attr(straw_verus,verus_spec(result=>ensures result==marked(*value)))]
pub fn marker(value: &Value) -> bool {
    crate::queue_roots::field_value(value, "type").as_str() == Some("StagingHandoff")
}

#[cfg_attr(straw_verus,verus_spec(result=>ensures result==unmarked(events@)))]
pub fn no_markers(events: &[Value]) -> bool {
    let mut i = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant i<=events.len(),forall|j:int| 0<=j<i ==> !marked(events@[j]),
        decreases events.len()-i,
    ))]
    while i < events.len() {
        if marker(&events[i]) {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg_attr(straw_verus,verus_spec(result=>
    ensures final(digester)@==old(digester)@,
        result.is_ok() ==> identity(result->Ok_0,prefix@,sequence,business@,old(digester)@),
))]
pub fn identify<D: Digest>(
    digester: &mut D,
    prefix: &str,
    sequence: u64,
    business: &[Value],
) -> Result<String, D::Error> {
    let preimage = envelope(prefix, sequence, business);
    let id = digester.digest(&preimage)?;
    #[cfg(straw_verus)]
    proof! {
        assert(exists|value:Value| value==preimage && package(value,prefix@,sequence,business@));
        assert forall|value:Value| package(value,prefix@,sequence,business@)
            implies id==(digester@)(crate::json_specs::content(value)) by {
            package_consistency(preimage,value,prefix@,sequence,business@,business@);
        }
    }
    Ok(id)
}

#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::environment::string_order_contract(),
    ensures marked(result),crate::json_specs::text(crate::queue_roots::field_or_null(result,"id"@))==Some(id@),
))]
pub fn marker_event(id: &str) -> Value {
    let mut object = serde_json::Map::new();
    object.insert(
        "type".to_owned(),
        Value::String("StagingHandoff".to_owned()),
    );
    object.insert("id".to_owned(), Value::String(id.to_owned()));
    Value::Object(object)
}

#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::environment::string_order_contract(),
    ensures final(digester)@==old(digester)@,
        match result {
            Err(_)=>final(events)@==old(events)@,
            Ok(None)=>references.len()==0 && final(events)@==old(events)@,
            Ok(Some(value))=>references.len()>0 && value.1==references
                && final(events)@.len()==old(events)@.len()+1
                && final(events)@.drop_last()==old(events)@
                && marked(final(events)@.last())
                && crate::json_specs::text(crate::queue_roots::field_or_null(final(events)@.last(),"id"@))==Some(value.0@)
                && identity(value.0,prefix@,sequence,old(events)@,old(digester)@),
        },
))]
pub fn attach<D: Digest>(
    digester: &mut D,
    prefix: &str,
    sequence: u64,
    events: &mut Vec<Value>,
    references: Vec<Value>,
) -> Result<Option<(String, Vec<Value>)>, D::Error> {
    if references.is_empty() {
        return Ok(None);
    }
    let id = identify(digester, prefix, sequence, events)?;
    events.push(marker_event(&id));
    Ok(Some((id, references)))
}

#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::environment::string_hash_contract(),
    ensures values_cover(result@,roots@),
))]
pub fn values(roots: HashMap<String, Value>) -> Vec<Value> {
    let mut it = crate::catalog_decode::owned_entries(roots);
    #[cfg(straw_verus)]
    let original = verus_exec_expr!(Ghost(crate::collection_specs::remaining(it)));
    let mut result = Vec::new();
    #[cfg_attr(straw_verus,verus_spec(
        invariant crate::environment::string_hash_contract(),
            original@.to_set()==roots@.kv_pairs(),original@.no_duplicates(),result.len()<=original@.len(),
            crate::collection_specs::remaining(it)==original@.skip(result@.len() as int),
            forall|i:int| 0<=i<result.len() ==> result@[i]==original@[i].1,
        decreases crate::collection_specs::remaining(it).len(),
    ))]
    loop {
        let Some((key, value)) = it.next() else {
            #[cfg(straw_verus)]
            proof! {
                assert(result@.len()==original@.len());
                assert forall|v:Value| result@.to_set().contains(v) implies roots@.values().contains(v) by {
                    let i=choose|i:int| 0<=i<result.len() && result@[i]==v;
                    assert(original@.to_set().contains(original@[i]));
                    assert(roots@.contains_key(original@[i].0));
                }
                assert forall|v:Value| roots@.values().contains(v) implies result@.to_set().contains(v) by {
                    let key=choose|key:String| roots@.contains_key(key) && roots@[key]==v;
                    assert(original@.to_set().contains((key,v)));
                    let i=choose|i:int| 0<=i<original@.len() && original@[i]==(key,v);
                    assert(result@[i]==v);
                }
                assert(result@.to_set() =~= roots@.values());
            }
            return result;
        };
        #[cfg(straw_verus)]
        proof! {assert((key,value)==original@[result@.len() as int]);}
        #[cfg(not(straw_verus))]
        let _ = key;
        result.push(value);
    }
}

#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::environment::string_hash_contract(),
    ensures final(digester)@==old(digester)@,
        result.is_ok() ==> crate::queue_roots::valid_keys(result->Ok_0@,old(digester)@)
            && result->Ok_0@.dom().to_iset()==crate::queue_roots::many_keys(business@,old(digester)@),
))]
pub fn roots<D: Digest>(
    digester: &mut D,
    business: &[Value],
) -> Result<HashMap<String, Value>, D::Error> {
    let mut output = HashMap::new();
    let mut i = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant crate::environment::string_hash_contract(),digester@==old(digester)@,i<=business.len(),
            crate::queue_roots::valid_keys(output@,digester@),
            output@.dom().to_iset()==crate::queue_roots::many_keys(business@.take(i as int),digester@),
        decreases business.len()-i,
    ))]
    while i < business.len() {
        crate::queue_roots::references(digester, &business[i], &mut output)?;
        #[cfg(straw_verus)]
        proof! {
            crate::queue_roots::many_push(business@.take(i as int),business@[i as int],digester@);
            assert(business@.take(i as int+1)==business@.take(i as int).push(business@[i as int]));
        }
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(business@.take(i as int)==business@);}
    Ok(output)
}

#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::environment::string_hash_contract(),
    ensures final(digester)@==old(digester)@,
        recovery_result(result,*transaction,prefix@,sequence,old(digester)@),
))]
#[allow(
    clippy::type_complexity,
    reason = "Keep component error types visible at the checked I/O boundary."
)]
pub fn recover<D: Digest>(
    digester: &mut D,
    prefix: &str,
    sequence: u64,
    transaction: &Value,
) -> Result<Option<(String, Vec<Value>)>, Error<D::Error>> {
    let Some(events) = transaction.as_array() else {
        return Err(Error::Transaction);
    };
    if events.is_empty() || !marker(&events[events.len() - 1]) {
        if !no_markers(events) {
            return Err(Error::Misplaced);
        }
        return Ok(None);
    }
    let business = &events[..events.len() - 1];
    if !no_markers(business) {
        return Err(Error::Repeated);
    }
    #[cfg(straw_verus)]
    proof! {
        assert(business@==events@.drop_last());
        assert(shaped(events@));
    }
    let id = match identify(digester, prefix, sequence, business) {
        Ok(id) => id,
        Err(error) => {
            #[cfg(straw_verus)]
            proof! {assert(recovery_result::<D::Error>(Err(Error::Digest(error)),*transaction,prefix@,sequence,digester@));}
            return Err(Error::Digest(error));
        }
    };
    if crate::queue_roots::field_value(&events[events.len() - 1], "id").as_str()
        != Some(id.as_str())
    {
        #[cfg(straw_verus)]
        proof! {
            assert(exists|computed:String| computed==id && identity(computed,prefix@,sequence,business@,digester@)
                && crate::json_specs::text(crate::queue_roots::field_or_null(events@.last(),"id"@))!=Some(computed@));
        }
        return Err(Error::Identity);
    }
    let found = match roots(digester, business) {
        Ok(found) => found,
        Err(error) => {
            #[cfg(straw_verus)]
            proof! {assert(recovery_result::<D::Error>(Err(Error::Digest(error)),*transaction,prefix@,sequence,digester@));}
            return Err(Error::Digest(error));
        }
    };
    if found.is_empty() {
        #[cfg(straw_verus)]
        proof! {assert(found@.dom() =~= Set::<String>::empty());assert(found@.dom().to_iset() =~= ISet::<String>::empty());}
        return Err(Error::Empty);
    }
    #[cfg(straw_verus)]
    let saved = verus_exec_expr!(Ghost(found@));
    let references = values(found);
    #[cfg(straw_verus)]
    proof! {
        assert(business@==events@.drop_last());
        assert(exists|roots:Map<String,Value>| roots==saved@ && values_cover(references@,roots)
            && roots.dom()!=Set::<String>::empty()
            && crate::queue_roots::valid_keys(roots,digester@)
            && roots.dom().to_iset()==crate::queue_roots::many_keys(events@.drop_last(),digester@));
    }
    Ok(Some((id, references)))
}

#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::environment::string_hash_contract(),
    ensures final(digester)@==old(digester)@,
        match result {
            Err(error)=>final(pending)@==old(pending)@ && recovery_result(Err(error),*transaction,prefix@,sequence,old(digester)@),
            Ok(())=>exists|outcome:Option<(String,Vec<Value>)>| recovery_result::<D::Error>(Ok(outcome),*transaction,prefix@,sequence,old(digester)@)
                && final(pending)@==match outcome {None=>old(pending)@,Some(value)=>old(pending)@.push(value)},
        },
))]
pub fn recover_into<D: Digest>(
    digester: &mut D,
    prefix: &str,
    sequence: u64,
    transaction: &Value,
    pending: &mut Vec<(String, Vec<Value>)>,
) -> Result<(), Error<D::Error>> {
    let outcome = recover(digester, prefix, sequence, transaction)?;
    #[cfg(straw_verus)]
    let saved = verus_exec_expr!(Ghost(outcome));
    if let Some(value) = outcome {
        pending.push(value);
    }
    #[cfg(straw_verus)]
    proof! {
        assert(exists|outcome:Option<(String,Vec<Value>)>| outcome==saved@ && recovery_result::<D::Error>(Ok(outcome),*transaction,prefix@,sequence,digester@)
            && pending@==match outcome {None=>old(pending)@,Some(value)=>old(pending)@.push(value)});
    }
    Ok(())
}
