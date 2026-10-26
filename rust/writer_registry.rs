//! Writer registry JSON decoding. Successful events retain the exact field
//! values and their frame order; the journal authenticates/parses raw frames.
use serde_json::Value;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub enum Event<'a> {
    Begin {
        sequence: u64,
        key: &'a str,
        owner: Option<&'a str>,
        dependencies: &'a Vec<Value>,
    },
    Results {
        operation: u64,
        roots: &'a Vec<Value>,
        intermediates: Option<&'a Vec<Value>>,
    },
    Done {
        operation: u64,
    },
    DoneOwner {
        owner: &'a str,
    },
    DoneKey {
        key: &'a str,
    },
    Retired {
        operations: Vec<u64>,
        results: &'a Vec<Value>,
    },
}
#[cfg(straw_verus)]
verus! {
    use crate::json_specs::{field,json,Json};
    pub open spec fn text(value:Value,key:Seq<char>)->Option<Seq<char>> {
        match field(value,key) {Some(v)=>match json(v) {Json::Text(s)=>Some(s),_=>None},None=>None}
    }
    pub open spec fn number(value:Value,key:Seq<char>)->Option<u64> {
        match field(value,key) {Some(v)=>match json(v) {Json::Number(n)=>n,_=>None},None=>None}
    }
    pub open spec fn array(value:Value,key:Seq<char>)->Option<Seq<Value>> {
        match field(value,key) {Some(v)=>match json(v) {Json::Array(s)=>Some(s),_=>None},None=>None}
    }
    pub open spec fn owner_matches(value:Value,owner:Option<&str>)->bool {
        match owner {None=>field(value,"owner"@).is_some() && json(field(value,"owner"@)->Some_0)==Json::Null,
            Some(s)=>text(value,"owner"@)==Some(s@)}
    }
    pub open spec fn integers(values:Seq<Value>)->bool {
        forall|i:int| 0 <= i < values.len() ==> match json(#[trigger] values[i]) {Json::Number(Some(_))=>true,_=>false}
    }
    pub open spec fn schema(value:Value)->bool {
        match text(value,"type"@) {None=>false,Some(kind)=>
            if kind=="Begin"@ {number(value,"sequence"@).is_some() && text(value,"key"@).is_some()
                && field(value,"owner"@).is_some() && (json(field(value,"owner"@)->Some_0)==Json::Null || text(value,"owner"@).is_some())
                && array(value,"dependencies"@).is_some()}
            else if kind=="Results"@ {number(value,"operation"@).is_some() && array(value,"results"@).is_some()
                && (field(value,"intermediates"@).is_none() || array(value,"intermediates"@).is_some())}
            else if kind=="Done"@ {number(value,"operation"@).is_some()}
            else if kind=="DoneOwner"@ {text(value,"owner"@).is_some()}
            else if kind=="DoneKey"@ {text(value,"key"@).is_some()}
            else if kind=="Retired"@ {array(value,"operations"@).is_some() && integers(array(value,"operations"@)->Some_0) && array(value,"results"@).is_some()}
            else {false}
        }
    }
    pub open spec fn frame_schema(value:Value)->bool {
        json(value) is Array && forall|i:int| 0 <= i < json(value)->Array_0.len() ==> schema(json(value)->Array_0[i])
    }
    pub open spec fn event_matches(value:Value,event:Event)->bool {
        match event {
            Event::Begin {sequence,key,owner,dependencies}=>text(value,"type"@)==Some("Begin"@)
                && number(value,"sequence"@)==Some(sequence) && text(value,"key"@)==Some(key@)
                && owner_matches(value,owner) && array(value,"dependencies"@)==Some(dependencies@),
            Event::Results {operation,roots,intermediates}=>text(value,"type"@)==Some("Results"@)
                && number(value,"operation"@)==Some(operation) && array(value,"results"@)==Some(roots@)
                && match intermediates {None=>field(value,"intermediates"@).is_none(),Some(values)=>array(value,"intermediates"@)==Some(values@)},
            Event::Done {operation}=>text(value,"type"@)==Some("Done"@) && number(value,"operation"@)==Some(operation),
            Event::DoneOwner {owner}=>text(value,"type"@)==Some("DoneOwner"@) && text(value,"owner"@)==Some(owner@),
            Event::DoneKey {key}=>text(value,"type"@)==Some("DoneKey"@) && text(value,"key"@)==Some(key@),
            Event::Retired {operations,results}=>text(value,"type"@)==Some("Retired"@) && array(value,"results"@)==Some(results@)
                && array(value,"operations"@).is_some() && array(value,"operations"@)->Some_0.len()==operations.len()
                && forall|i:int| 0 <= i < operations.len() ==> json(array(value,"operations"@)->Some_0[i])==Json::Number(Some(operations@[i])),
        }
    }
    pub open spec fn frame_matches(value:Value,events:Seq<Event>)->bool {
        json(value) is Array && events.len()==json(value)->Array_0.len()
            && forall|i:int| 0 <= i < events.len() ==> event_matches(json(value)->Array_0[i],events[i])
    }
    pub open spec fn raw_matches(raw:Seq<u8>,events:Seq<Event>)->bool {
        exists|value:Value| crate::json_specs::parses(raw,value) && frame_matches(value,events)
    }
    pub proof fn journal_to_registry(raw:Seq<u8>,value:Value,events:Seq<Event>)
        requires crate::json_specs::parses(raw,value),frame_matches(value,events),
        ensures raw_matches(raw,events),
    {}
}
#[cfg_attr(straw_verus,verus_spec(result =>
    ensures result.is_some() == text(*value,key@).is_some(), result.is_some() ==> result->Some_0@ == text(*value,key@)->Some_0,
))]
fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    crate::catalog_json::get(value, key)?.as_str()
}
#[cfg_attr(straw_verus,verus_spec(result => ensures result == number(*value,key@)))]
fn integer(value: &Value, key: &str) -> Option<u64> {
    crate::catalog_json::get(value, key)?.as_u64()
}
#[cfg_attr(straw_verus,verus_spec(result =>
    ensures result.is_some() == array(*value,key@).is_some(), result.is_some() ==> result->Some_0@ == array(*value,key@)->Some_0,
))]
fn values<'a>(value: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    crate::catalog_json::get(value, key)?.as_array()
}
#[cfg_attr(straw_verus,verus_spec(result => ensures result.is_some() == schema(*value), result.is_some() ==> event_matches(*value,result->Some_0)))]
pub fn decode(value: &Value) -> Option<Event<'_>> {
    let kind = string(value, "type")?;
    if kind == "Begin" {
        let sequence = integer(value, "sequence")?;
        let key = string(value, "key")?;
        let v = crate::catalog_json::get(value, "owner")?;
        let owner = if v.is_null() { None } else { Some(v.as_str()?) };
        Some(Event::Begin {
            sequence,
            key,
            owner,
            dependencies: values(value, "dependencies")?,
        })
    } else if kind == "Results" {
        let operation = integer(value, "operation")?;
        let roots = values(value, "results")?;
        let intermediates = match crate::catalog_json::get(value, "intermediates") {
            None => None,
            Some(v) => Some(v.as_array()?),
        };
        Some(Event::Results {
            operation,
            roots,
            intermediates,
        })
    } else if kind == "Done" {
        Some(Event::Done {
            operation: integer(value, "operation")?,
        })
    } else if kind == "DoneOwner" {
        Some(Event::DoneOwner {
            owner: string(value, "owner")?,
        })
    } else if kind == "DoneKey" {
        Some(Event::DoneKey {
            key: string(value, "key")?,
        })
    } else if kind == "Retired" {
        let operations = values(value, "operations")?;
        let results = values(value, "results")?;
        let mut decoded = Vec::new();
        let mut index = 0;
        #[cfg_attr(straw_verus,verus_spec(
            invariant index <= operations.len(),decoded.len()==index,
                text(*value,"type"@)==Some("Retired"@),
                array(*value,"operations"@)==Some(operations@),
                forall|i:int| 0 <= i < index ==> crate::json_specs::json(operations@[i])==crate::json_specs::Json::Number(Some(decoded@[i])),
            decreases operations.len()-index,
        ))]
        while index < operations.len() {
            let number = match operations[index].as_u64() {
                Some(number) => number,
                None => {
                    #[cfg(straw_verus)]
                    proof! {assert(!integers(operations@));assert(!schema(*value));}
                    return None;
                }
            };
            decoded.push(number);
            index += 1;
        }
        Some(Event::Retired {
            operations: decoded,
            results,
        })
    } else {
        None
    }
}
#[cfg_attr(straw_verus,verus_spec(result => ensures result.is_some() == frame_schema(*value), result.is_some() ==> frame_matches(*value,result->Some_0@)))]
pub fn decode_frame(value: &Value) -> Option<Vec<Event<'_>>> {
    let values = value.as_array()?;
    let mut result = Vec::new();
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index <= values.len(),result.len()==index,
            crate::json_specs::json(*value)==crate::json_specs::Json::Array(values@),
            forall|i:int| 0 <= i < index ==> event_matches(values@[i],result@[i]) && schema(values@[i]),
        decreases values.len()-index,
    ))]
    while index < values.len() {
        let event = match decode(&values[index]) {
            Some(event) => event,
            None => {
                #[cfg(straw_verus)]
                proof! {
                    reveal(frame_schema);
                    assert(!schema(values@[index as int]));
                    if frame_schema(*value) { assert(schema(crate::json_specs::json(*value)->Array_0[index as int])); }
                    assert(!frame_schema(*value));
                }
                return None;
            }
        };
        result.push(event);
        index += 1;
    }
    Some(result)
}

pub type Evidence = crate::writer_retirement::Evidence<Vec<Value>>;
#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(not(straw_verus), derive(Clone, PartialEq))]
pub struct Results {
    pub roots: Vec<Value>,
    pub intermediates: Vec<Value>,
}
pub type Operation = crate::writer_retirement::Operation<Evidence, Results>;

#[cfg(straw_verus)]
verus! {
    pub open spec fn copied_values(before:Seq<Value>,after:Seq<Value>)->bool {
        before.len()==after.len() && forall|i:int| 0 <= i < before.len() ==>
            crate::json_specs::content(before[i])==crate::json_specs::content(after[i])
            && (forall|depth:nat| crate::json_specs::observe(before[i],depth)==crate::json_specs::observe(after[i],depth))
    }
    pub open spec fn owner_view(owner:Option<String>)->Option<Seq<char>> {
        match owner {None=>None,Some(value)=>Some(value@)}
    }
    pub open spec fn borrowed_owner(owner:Option<&str>)->Option<Seq<char>> {
        match owner {None=>None,Some(value)=>Some(value@)}
    }
    pub open spec fn preserved(before:Seq<Operation>,after:Seq<Operation>)->bool {
        before.len() <= after.len() && forall|i:int| 0 <= i < before.len() ==>
            after[i].sequence==before[i].sequence && after[i].evidence==before[i].evidence
            && (before[i].results.is_some() ==> after[i].results==before[i].results)
            && (before[i].done ==> after[i].done)
    }
    pub open spec fn equal_sequences(left:Seq<Value>,right:Seq<Value>)->bool {
        left.len()==right.len() && forall|i:int| 0 <= i < left.len() ==> crate::json_specs::equal(left[i],right[i])
    }
    pub open spec fn describable(operations:Seq<Operation>,sequence:u64,roots:Seq<Value>,intermediates:Seq<Value>)->bool {
        exists|i:int| 0 <= i < operations.len() && (#[trigger] operations[i]).sequence==sequence
            && (forall|j:int| 0 <= j < i ==> operations[j].sequence!=sequence)
            && !operations[i].done && match operations[i].results {
                None=>true,Some(previous)=>equal_sequences(previous.roots@,roots) && equal_sequences(previous.intermediates@,intermediates),
            }
    }
    pub open spec fn ids_known(operations:Seq<Operation>,ids:Seq<u64>)->bool {
        forall|id:u64| ids.contains(id) ==> exists|i:int| 0 <= i < operations.len() && (#[trigger] operations[i]).sequence==id
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures copied_values(values@,result@)))]
fn copy_values(values: &[Value]) -> Vec<Value> {
    let mut result = Vec::with_capacity(values.len());
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index <= values.len(),result.len()==index,
            forall|i:int| 0 <= i < index ==> crate::json_specs::content(values@[i])==crate::json_specs::content(result@[i])
                && (forall|depth:nat| crate::json_specs::observe(values@[i],depth)==crate::json_specs::observe(result@[i],depth)),
        decreases values.len()-index,
    ))]
    while index < values.len() {
        result.push(values[index].clone());
        index += 1;
    }
    result
}
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures result == (frame==sequence && (old(operations).len()==0 || old(operations)@[old(operations).len()-1].sequence<frame)),
        result ==> final(operations).len()==old(operations).len()+1,
        result ==> final(operations)@.drop_last()==old(operations)@,
        result ==> final(operations)@.last().sequence==frame,
        result ==> final(operations)@.last().evidence.key@==key@,
        result ==> owner_view(final(operations)@.last().evidence.owner)==borrowed_owner(owner),
        result ==> copied_values(dependencies@,final(operations)@.last().evidence.dependencies@),
        result ==> final(operations)@.last().results.is_none() && !final(operations)@.last().done,
        result ==> frame==sequence,
        !result ==> final(operations)@==old(operations)@,
        preserved(old(operations)@,final(operations)@),
))]
#[allow(
    clippy::manual_map,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn begin(
    operations: &mut Vec<Operation>,
    frame: u64,
    sequence: u64,
    key: &str,
    owner: Option<&str>,
    dependencies: &[Value],
) -> bool {
    if frame != sequence {
        return false;
    }
    let owner = match owner {
        Some(value) => Some(value.to_owned()),
        None => None,
    };
    let evidence = Evidence {
        key: key.to_owned(),
        owner,
        dependencies: copy_values(dependencies),
    };
    crate::writer_retirement::register_operation(operations, frame, evidence)
}
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures result == equal_sequences(old_values@,incoming@), result ==> old_values.len()==incoming.len(),
        result ==> forall|i:int,depth:nat| 0 <= i < old_values.len() ==> crate::json_specs::observe(old_values@[i],depth)==crate::json_specs::observe(incoming@[i],depth),
))]
fn equal_values(old_values: &[Value], incoming: &[Value]) -> bool {
    if old_values.len() != incoming.len() {
        return false;
    }
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=old_values.len(),old_values.len()==incoming.len(),
            forall|i:int| 0 <= i < index ==> crate::json_specs::equal(old_values@[i],incoming@[i]),
            forall|i:int,depth:nat| 0 <= i < index ==> crate::json_specs::observe(old_values@[i],depth)==crate::json_specs::observe(incoming@[i],depth),
        decreases old_values.len()-index,
    ))]
    while index < old_values.len() {
        if old_values[index] != incoming[index] {
            #[cfg(straw_verus)]
            proof! {assert(!equal_sequences(old_values@,incoming@));}
            return false;
        }
        index += 1;
    }
    true
}
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures result == describable(old(operations)@,sequence,roots@,intermediates@),
        preserved(old(operations)@,final(operations)@),final(operations).len()==old(operations).len(),
        !result ==> final(operations)@==old(operations)@,
        forall|i:int| 0 <= i < old(operations).len() ==> final(operations)@[i].done==old(operations)@[i].done,
        forall|i:int| 0 <= i < old(operations).len() && old(operations)@[i].sequence!=sequence ==> final(operations)@[i]==old(operations)@[i],
        result ==> exists|i:int| 0 <= i < old(operations).len() && (#[trigger] old(operations)@[i]).sequence==sequence
            && !old(operations)@[i].done && final(operations)@[i].results.is_some()
            && (old(operations)@[i].results.is_none() ==> copied_values(roots@,final(operations)@[i].results->Some_0.roots@)
                && copied_values(intermediates@,final(operations)@[i].results->Some_0.intermediates@)),
))]
#[allow(
    clippy::ptr_arg,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn describe(
    operations: &mut Vec<Operation>,
    sequence: u64,
    roots: &[Value],
    intermediates: &[Value],
) -> bool {
    let index = match crate::writer_retirement::operation_position(operations, sequence) {
        Some(index) => index,
        None => return false,
    };
    if operations[index].done {
        return false;
    }
    if let Some(previous) = &operations[index].results {
        return equal_values(&previous.roots, roots)
            && equal_values(&previous.intermediates, intermediates);
    }
    let results = Results {
        roots: copy_values(roots),
        intermediates: copy_values(intermediates),
    };
    crate::writer_retirement::remember_results(&mut operations[index], results, true)
}

#[cfg_attr(straw_verus,verus_spec(result=>
    ensures final(operations).len()==old(operations).len(),
        !result ==> final(operations)@==old(operations)@,
        result ==> forall|i:int| 0 <= i < old(operations).len() ==> final(operations)@[i] == (Operation {
            done:old(operations)@[i].done || ids@.contains(old(operations)@[i].sequence),..old(operations)@[i]
        }),
        result == ids_known(old(operations)@,ids@),
        preserved(old(operations)@,final(operations)@),
))]
#[allow(
    clippy::ptr_arg,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn settle_ids(operations: &mut Vec<Operation>, ids: &[u64]) -> bool {
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=ids.len(),operations@==old(operations)@,
            forall|j:int| 0 <= j < index ==> exists|i:int| 0 <= i < operations.len() && (#[trigger] operations@[i]).sequence==(#[trigger] ids@[j]),
        decreases ids.len()-index,
    ))]
    while index < ids.len() {
        if crate::writer_retirement::operation_position(operations, ids[index]).is_none() {
            #[cfg(straw_verus)]
            proof! {assert(ids@.contains(ids@[index as int]));assert(!ids_known(operations@,ids@));}
            return false;
        }
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! {
        reveal(ids_known);
        assert(index==ids.len());
        assert forall|id:u64| ids@.contains(id) implies exists|i:int| 0 <= i < operations.len() && (#[trigger] operations@[i]).sequence==id by {
            let j=choose|j:int| 0 <= j < ids.len() && ids@[j]==id;
            assert(exists|i:int| 0 <= i < operations.len() && (#[trigger] operations@[i]).sequence==id);
        }
        assert(ids_known(operations@,ids@));
    }
    index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=operations.len(),operations.len()==old(operations).len(),
            ids_known(old(operations)@,ids@),
            forall|i:int| 0 <= i < operations.len() ==> operations@[i] == (Operation {
                done:old(operations)@[i].done || (i<index && ids@.contains(old(operations)@[i].sequence)),..old(operations)@[i]
            }),
        decreases operations.len()-index,
    ))]
    while index < operations.len() {
        if ids.contains(&operations[index].sequence) {
            crate::writer_retirement::settle_operation(&mut operations[index]);
        }
        index += 1;
    }
    true
}
#[cfg(straw_verus)]
verus! {
    pub open spec fn value_covered(values:Seq<Value>,value:Value)->bool {
        exists|i:int| 0 <= i < values.len() && (forall|depth:nat| crate::json_specs::observe(#[trigger] values[i],depth)==crate::json_specs::observe(value,depth))
    }
    pub open spec fn receipt_preserved(before:Seq<Value>,after:Seq<Value>)->bool {
        before.len()<=after.len() && forall|i:int| 0 <= i < before.len() ==> after[i]==before[i]
    }
    pub open spec fn receipt_source(value:Value,incoming:Seq<Value>)->bool {
        exists|j:int| 0 <= j < incoming.len() && crate::json_specs::content(value)==crate::json_specs::content(#[trigger] incoming[j])
    }
    pub open spec fn receipt_sources(before:Seq<Value>,after:Seq<Value>,incoming:Seq<Value>)->bool {
        after.len()<=before.len()+incoming.len() && forall|i:int| before.len() <= i < after.len() ==>
            receipt_source(#[trigger] after[i],incoming)
    }
}
#[cfg_attr(straw_verus,verus_spec(result=> ensures result ==> value_covered(values@,*value)))]
fn contains_value(values: &[Value], value: &Value) -> bool {
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(invariant index<=values.len(),decreases values.len()-index))]
    while index < values.len() {
        if values[index] == *value {
            #[cfg(straw_verus)]
            proof! {assert(value_covered(values@,*value));}
            return true;
        }
        index += 1;
    }
    false
}
#[cfg(straw_verus)]
verus! {
    pub open spec fn receipt_step(before:Seq<Value>,after:Seq<Value>,value:Value)->bool {
        after==before || (after.len()==before.len()+1 && after.drop_last()==before
            && crate::json_specs::content(after.last())==crate::json_specs::content(value))
    }
    pub proof fn receipt_sources_extend(original:Seq<Value>,before:Seq<Value>,after:Seq<Value>,incoming:Seq<Value>,count:int)
        requires 0 <= count < incoming.len(),receipt_sources(original,before,incoming.take(count)),
            receipt_step(before,after,incoming[count]),
        ensures receipt_sources(original,after,incoming.take(count+1)),
    {
        assert(incoming.take(count).len()==count);
        assert(incoming.take(count+1).len()==count+1);
        assert(after.len()<=before.len()+1);
        assert(after.len()<=original.len()+incoming.take(count+1).len());
        assert forall|i:int| original.len() <= i < after.len() implies receipt_source(after[i],incoming.take(count+1)) by {
            if i<before.len() {
                let j=choose|j:int| 0 <= j < incoming.take(count).len()
                    && crate::json_specs::content(before[i])==crate::json_specs::content(#[trigger] incoming.take(count)[j]);
                assert(after[i]==before[i]);
                assert(incoming.take(count+1)[j]==incoming.take(count)[j]);
                assert(receipt_source(after[i],incoming.take(count+1)));
            } else {
                assert(i==before.len());assert(after[i]==after.last());
                assert(incoming.take(count+1)[count]==incoming[count]);
                assert(receipt_source(after[i],incoming.take(count+1)));
            }
        }
    }
}
#[cfg_attr(straw_verus,verus_spec(
    ensures receipt_preserved(old(kept)@,final(kept)@),receipt_step(old(kept)@,final(kept)@,*value),
        value_covered(final(kept)@,*value),
))]
fn keep_one(kept: &mut Vec<Value>, value: &Value) {
    if !contains_value(kept, value) {
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(kept@));
        kept.push(value.clone());
        #[cfg(straw_verus)]
        proof! {
            assert(kept@.drop_last() =~= before@);
            assert forall|depth:nat| crate::json_specs::observe(kept@[before@.len() as int],depth)==crate::json_specs::observe(*value,depth) by {};
            assert(exists|i:int| i==before@.len() && 0 <= i < kept.len() && (forall|depth:nat| crate::json_specs::observe(#[trigger] kept@[i],depth)==crate::json_specs::observe(*value,depth)));
        }
    }
}
#[cfg_attr(straw_verus,verus_spec(
    ensures receipt_preserved(old(kept)@,final(kept)@),
        receipt_sources(old(kept)@,final(kept)@,incoming@),
        forall|i:int| 0 <= i < incoming.len() ==> value_covered(final(kept)@,incoming@[i]),
))]
pub fn keep_results(kept: &mut Vec<Value>, incoming: &[Value]) {
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=incoming.len(),receipt_preserved(old(kept)@,kept@),
            receipt_sources(old(kept)@,kept@,incoming@.take(index as int)),
            forall|i:int| 0 <= i < index ==> value_covered(kept@,incoming@[i]),
        decreases incoming.len()-index,
    ))]
    while index < incoming.len() {
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(kept@));
        keep_one(kept, &incoming[index]);
        #[cfg(straw_verus)]
        proof! {
            receipt_sources_extend(old(kept)@,before@,kept@,incoming@,index as int);
            assert forall|i:int| 0 <= i < index implies value_covered(kept@,incoming@[i]) by {
                let j=choose|j:int| 0 <= j < before@.len() && (forall|depth:nat| crate::json_specs::observe(#[trigger] before@[j],depth)==crate::json_specs::observe(incoming@[i],depth));
                assert(kept@[j]==before@[j]);
            }
        }
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(incoming@.take(index as int) =~= incoming@);}
}

#[cfg(straw_verus)]
verus! {
    pub type State=(Seq<Operation>,Seq<Value>);
    pub open spec fn optional_values(values:Option<&Vec<Value>>)->Seq<Value> {
        match values {None=>Seq::empty(),Some(values)=>values@}
    }
    pub open spec fn admitted(operations:Seq<Operation>,frame:u64,event:Event)->bool {
        match event {
            Event::Begin {sequence,..}=>frame==sequence && (operations.len()==0 || operations.last().sequence<frame),
            Event::Results {operation,roots,intermediates}=>describable(operations,operation,roots@,optional_values(intermediates)),
            Event::Done {operation}=>ids_known(operations,seq![operation]),
            Event::DoneOwner {..}|Event::DoneKey {..}=>true,
            Event::Retired {operations:ids,..}=>ids_known(operations,ids@),
        }
    }
    pub open spec fn settled(before:Seq<Operation>,after:Seq<Operation>,ids:Seq<u64>)->bool {
        before.len()==after.len() && forall|i:int| 0 <= i < before.len() ==> after[i]==(Operation {done:before[i].done || ids.contains(before[i].sequence),..before[i]})
    }
    pub open spec fn applied(before:State,after:State,frame:u64,event:Event)->bool {
        preserved(before.0,after.0) && admitted(before.0,frame,event) && match event {
            Event::Begin {sequence,key,owner,dependencies}=>after.1==before.1 && after.0.len()==before.0.len()+1
                && after.0.drop_last()==before.0 && after.0.last().sequence==frame && frame==sequence
                && after.0.last().evidence.key@==key@ && owner_view(after.0.last().evidence.owner)==borrowed_owner(owner)
                && copied_values(dependencies@,after.0.last().evidence.dependencies@)
                && after.0.last().results.is_none() && !after.0.last().done,
            Event::Results {operation,roots,intermediates}=>after.1==before.1 && before.0.len()==after.0.len()
                && (forall|i:int| 0 <= i < before.0.len() ==> after.0[i].done==before.0[i].done)
                && (forall|i:int| 0 <= i < before.0.len() && before.0[i].sequence!=operation ==> after.0[i]==before.0[i])
                && (exists|i:int| 0 <= i < before.0.len() && (#[trigger] before.0[i]).sequence==operation && !before.0[i].done && after.0[i].results.is_some()
                    && (before.0[i].results.is_none() ==> copied_values(roots@,after.0[i].results->Some_0.roots@)
                        && copied_values(optional_values(intermediates),after.0[i].results->Some_0.intermediates@))),
            Event::Done {operation}=>after.1==before.1 && settled(before.0,after.0,seq![operation]),
            Event::DoneOwner {owner}=>after.1==before.1 && before.0.len()==after.0.len()
                && forall|i:int| 0 <= i < before.0.len() ==> after.0[i]==(Operation {done:before.0[i].done || crate::writer_retirement::named(before.0[i].evidence,owner@,true),..before.0[i]}),
            Event::DoneKey {key}=>after.1==before.1 && before.0.len()==after.0.len()
                && forall|i:int| 0 <= i < before.0.len() ==> after.0[i]==(Operation {done:before.0[i].done || crate::writer_retirement::named(before.0[i].evidence,key@,false),..before.0[i]}),
            Event::Retired {operations:ids,results}=>settled(before.0,after.0,ids@) && receipt_preserved(before.1,after.1) && receipt_sources(before.1,after.1,results@)
                && forall|i:int| 0 <= i < results.len() ==> value_covered(after.1,results@[i]),
        }
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures result==admitted(old(operations)@,frame,*event),
        result ==> applied((old(operations)@,old(receipts)@),(final(operations)@,final(receipts)@),frame,*event),
        !result ==> final(operations)@==old(operations)@ && final(receipts)@==old(receipts)@,
))]
#[allow(
    clippy::needless_return,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn apply_event(
    operations: &mut Vec<Operation>,
    receipts: &mut Vec<Value>,
    frame: u64,
    event: &Event<'_>,
) -> bool {
    #[cfg(straw_verus)]
    proof! {reveal(admitted);reveal(applied);}
    match event {
        Event::Begin {
            sequence,
            key,
            owner,
            dependencies,
        } => return begin(operations, frame, *sequence, key, *owner, dependencies),
        Event::Results {
            operation,
            roots,
            intermediates,
        } => {
            let intermediate_refs: &[Value] = match intermediates {
                None => &[],
                Some(values) => values.as_slice(),
            };
            return describe(operations, *operation, roots, intermediate_refs);
        }
        Event::Done { operation } => {
            let ids = [*operation];
            #[cfg(straw_verus)]
            proof! {assert(ids@ =~= seq![*operation]);}
            return settle_ids(operations, &ids);
        }
        Event::DoneOwner { owner } => {
            crate::writer_retirement::settle_named(operations, owner, true);
            return true;
        }
        Event::DoneKey { key } => {
            crate::writer_retirement::settle_named(operations, key, false);
            return true;
        }
        Event::Retired {
            operations: ids,
            results,
        } => {
            if !settle_ids(operations, ids) {
                return false;
            }
            keep_results(receipts, results);
            return true;
        }
    }
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn event_path(states:Seq<State>,frame:u64,events:Seq<Event>)->bool {
        states.len()==events.len()+1 && forall|i:int| 0 <= i < events.len() ==> applied(states[i],states[i+1],frame,events[i])
    }
    pub open spec fn frame_applied(before:State,after:State,frame:u64,value:Value)->bool {
        exists|events:Seq<Event>,states:Seq<State>| frame_matches(value,events) && event_path(states,frame,events)
            && states[0]==before && states.last()==after
    }
    pub proof fn event_path_push(states:Seq<State>,frame:u64,events:Seq<Event>,after:State,event:Event)
        requires event_path(states,frame,events),applied(states.last(),after,frame,event),
        ensures event_path(states.push(after),frame,events.push(event)),
    {
        assert forall|i:int| 0 <= i < events.push(event).len() implies applied(states.push(after)[i],states.push(after)[i+1],frame,events.push(event)[i]) by {
            if i<events.len() {assert(applied(states[i],states[i+1],frame,events[i]));}
        }
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures result ==> frame_applied((old(operations)@,old(receipts)@),(final(operations)@,final(receipts)@),frame,*value),
))]
pub fn apply_frame(
    operations: &mut Vec<Operation>,
    receipts: &mut Vec<Value>,
    frame: u64,
    value: &Value,
) -> bool {
    let events = match decode_frame(value) {
        Some(events) => events,
        None => return false,
    };
    #[cfg(straw_verus)]
    let mut states = verus_exec_expr!(Ghost(Seq::<State>::empty().push((operations@,receipts@))));
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=events.len(),frame_matches(*value,events@),
            event_path(states@,frame,events@.take(index as int)),
            states@[0]==(old(operations)@,old(receipts)@),
            states@.last()==(operations@,receipts@),
        decreases events.len()-index,
    ))]
    while index < events.len() {
        if !apply_event(operations, receipts, frame, &events[index]) {
            return false;
        }
        #[cfg(straw_verus)]
        proof! {
            event_path_push(states@,frame,events@.take(index as int),(operations@,receipts@),events@[index as int]);
            assert(events@.take(index as int).push(events@[index as int]) =~= events@.take(index as int+1));
            let next=states@.push((operations@,receipts@)); *states.borrow_mut()=next;
        }
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! {
        assert(events@.take(index as int) =~= events@);
        assert(frame_applied((old(operations)@,old(receipts)@),(operations@,receipts@),frame,*value));
    }
    true
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn history_path(states:Seq<State>,frames:Seq<Value>)->bool {
        frames.len()>0 && states.len()==frames.len() && states[0]==(Seq::<Operation>::empty(),Seq::<Value>::empty())
            && forall|i:int| 1 <= i < frames.len() ==> frame_applied(states[i-1],states[i],i as u64,frames[i])
    }
    pub open spec fn replayed(frames:Seq<Value>,state:State)->bool {
        exists|states:Seq<State>| history_path(states,frames) && states.last()==state
    }
    pub open spec fn raw_frame_applied(before:State,after:State,frame:u64,raw:Seq<u8>)->bool {
        exists|value:Value| crate::json_specs::parses(raw,value) && frame_applied(before,after,frame,value)
    }
    pub open spec fn raw_replayed(raw:Seq<Seq<u8>>,state:State)->bool {
        exists|states:Seq<State>| raw.len()>0 && states.len()==raw.len()
            && states[0]==(Seq::<Operation>::empty(),Seq::<Value>::empty()) && states.last()==state
            && (forall|i:int| 1 <= i < raw.len() ==> raw_frame_applied(states[i-1],states[i],i as u64,raw[i]))
    }
    pub proof fn journal_history_to_registry(raw:Seq<Seq<u8>>,frames:Seq<Value>,state:State)
        requires crate::journal_recovery::decoded(raw,frames),replayed(frames,state),
        ensures raw_replayed(raw,state),
    {
        let states=choose|states:Seq<State>| history_path(states,frames) && states.last()==state;
        assert forall|i:int| 1 <= i < raw.len() implies raw_frame_applied(states[i-1],states[i],i as u64,raw[i]) by {
            assert(crate::json_specs::parses(raw[i],frames[i]));
            assert(frame_applied(states[i-1],states[i],i as u64,frames[i]));
        }
        assert(raw_replayed(raw,state));
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>
    requires old(operations).len()==0,old(receipts).len()==0,frames.len()<=u64::MAX,
    ensures result ==> replayed(frames@,(final(operations)@,final(receipts)@)),
))]
pub fn replay(
    operations: &mut Vec<Operation>,
    receipts: &mut Vec<Value>,
    frames: &[Value],
) -> bool {
    if frames.is_empty() {
        return false;
    }
    #[cfg(straw_verus)]
    proof! {assert(operations@ =~= Seq::<Operation>::empty());assert(receipts@ =~= Seq::<Value>::empty());}
    #[cfg(straw_verus)]
    let mut states = verus_exec_expr!(Ghost(Seq::<State>::empty().push((operations@,receipts@))));
    let mut frame = 1;
    #[cfg_attr(straw_verus,verus_spec(
        invariant 1 <= frame <= frames.len(),frames.len()<=u64::MAX,
            history_path(states@,frames@.take(frame as int)),states@.last()==(operations@,receipts@),
        decreases frames.len()-frame,
    ))]
    while frame < frames.len() {
        if !apply_frame(operations, receipts, frame as u64, &frames[frame]) {
            return false;
        }
        #[cfg(straw_verus)]
        proof! {
            let extended=states@.push((operations@,receipts@));
            assert forall|i:int| 1 <= i < frame+1 implies frame_applied(extended[i-1],extended[i],i as u64,frames@[i]) by {
                if i<frame {
                    assert(frame_applied(states@[i-1],states@[i],i as u64,frames@.take(frame as int)[i]));
                    assert(frame_applied(states@[i-1],states@[i],i as u64,frames@[i]));
                }
            }
            *states.borrow_mut()=extended;
        }
        frame += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(frames@.take(frame as int) =~= frames@);assert(replayed(frames@,(operations@,receipts@)));}
    true
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn has_receipt(operation:Operation,key:Seq<char>)->bool {
        operation.evidence.key@==key && operation.results.is_some()
    }
    pub open spec fn latest_receipt(operations:Seq<Operation>,key:Seq<char>,index:int)->bool {
        0 <= index < operations.len() && has_receipt(operations[index],key)
            && forall|later:int| index < later < operations.len() ==> !has_receipt(operations[later],key)
    }
}
/// Settlement is deliberately irrelevant: Done records the internal handoff,
/// not delivery of the reply to the caller. An unfinished later attempt also
/// cannot hide a result already recorded for the same exact API/request key.
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures match result {
        Some(roots)=>exists|index:int| latest_receipt(operations@,key@,index)
            && roots@==operations@[index].results->Some_0.roots@,
        None=>forall|index:int| 0 <= index < operations.len() ==> !has_receipt(operations@[index],key@),
    },
))]
#[allow(
    clippy::collapsible_if,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn receipt<'a>(operations: &'a [Operation], key: &str) -> Option<&'a Vec<Value>> {
    let mut index = operations.len();
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=operations.len(),
            forall|later:int| index <= later < operations.len() ==> !has_receipt(operations@[later],key@),
        decreases index,
    ))]
    while index > 0 {
        index -= 1;
        if operations[index].evidence.key.as_str() == key {
            if let Some(results) = &operations[index].results {
                #[cfg(straw_verus)]
                proof! { assert(latest_receipt(operations@,key@,index as int)); }
                return Some(&results.roots);
            }
        }
    }
    None
}

#[cfg(straw_verus)]
verus! {
    pub enum RecoveryAction {
        Validate(Seq<Value>),
        Stage(Seq<Value>),
        Consume(Seq<char>,Seq<Value>),
        Release(Seq<char>),
        Commit(Seq<u64>,Seq<Value>),
    }
    pub trait RecoveryModel: View<V=Seq<RecoveryAction>> {}
    pub open spec fn action_prefix(actual:Seq<RecoveryAction>,planned:Seq<RecoveryAction>)->bool {
        actual.len()<=planned.len() && forall|i:int| 0 <= i < actual.len() ==> actual[i]==planned[i]
    }
    pub proof fn action_prefix_extend(actual:Seq<RecoveryAction>,prefix:Seq<RecoveryAction>,suffix:Seq<RecoveryAction>)
        requires action_prefix(actual,prefix),
        ensures action_prefix(actual,prefix+suffix),
    {}
    pub proof fn action_prefix_transitive(first:Seq<RecoveryAction>,second:Seq<RecoveryAction>,third:Seq<RecoveryAction>)
        requires action_prefix(first,second),action_prefix(second,third),
        ensures action_prefix(first,third),
    {}
    pub proof fn action_prefix_context(context:Seq<RecoveryAction>,first:Seq<RecoveryAction>,second:Seq<RecoveryAction>)
        requires action_prefix(first,second),
        ensures action_prefix(context+first,context+second),
    {}
    pub open spec fn completed(before:Seq<RecoveryAction>,after:Seq<RecoveryAction>,action:RecoveryAction,success:bool)->bool {
        (after==before || after==before.push(action)) && (success ==> after==before.push(action))
    }
    pub open spec fn handle_actions(operation:Operation,recover:bool)->Seq<RecoveryAction> {
        if recover && operation.results.is_some() {
            seq![RecoveryAction::Validate(operation.results->Some_0.roots@),RecoveryAction::Stage(operation.results->Some_0.roots@)]
        } else { Seq::empty() }
    }
    pub open spec fn release_actions(operation:Operation)->Seq<RecoveryAction> {
        (if operation.results.is_some() && operation.results->Some_0.intermediates.len()>0 {
            seq![RecoveryAction::Consume(operation.evidence.key@,operation.results->Some_0.intermediates@)]
        } else { Seq::empty() })
        +(match operation.evidence.owner {Some(owner)=>seq![RecoveryAction::Release(owner@)],None=>Seq::empty()})
    }
    pub open spec fn handles(operations:Seq<Operation>,recover:bool)->Seq<RecoveryAction>
        decreases operations.len(),
    { if operations.len()==0 {Seq::empty()} else {handles(operations.drop_last(),recover)+handle_actions(operations.last(),recover)} }
    pub open spec fn recovered_roots(operation:Operation,recover:bool)->Seq<Value> {
        if recover && operation.results.is_some() {operation.results->Some_0.roots@} else {Seq::empty()}
    }
    pub open spec fn recovery_roots(operations:Seq<Operation>,recover:bool)->Seq<Value>
        decreases operations.len(),
    { if operations.len()==0 {Seq::empty()} else {recovery_roots(operations.drop_last(),recover)+recovered_roots(operations.last(),recover)} }
    pub open spec fn releases(operations:Seq<Operation>)->Seq<RecoveryAction>
        decreases operations.len(),
    { if operations.len()==0 {Seq::empty()} else {releases(operations.drop_last())+release_actions(operations.last())} }
    pub open spec fn sequences(operations:Seq<Operation>)->Seq<u64> {
        operations.map(|index:int,operation:Operation| operation.sequence)
    }
    pub proof fn plan_extend(operations:Seq<Operation>,index:int,recover:bool)
        requires 0 <= index < operations.len(),
        ensures handles(operations.take(index+1),recover)==handles(operations.take(index),recover)+handle_actions(operations[index],recover),
            releases(operations.take(index+1))==releases(operations.take(index))+release_actions(operations[index]),
            recovery_roots(operations.take(index+1),recover)==recovery_roots(operations.take(index),recover)+recovered_roots(operations[index],recover),
    {
        assert(operations.take(index+1).drop_last() =~= operations.take(index));
        assert(operations.take(index+1).last()==operations[index]);
    }
    pub proof fn receipt_sources_join(original:Seq<Value>,before:Seq<Value>,after:Seq<Value>,first:Seq<Value>,second:Seq<Value>)
        requires receipt_preserved(original,before),receipt_preserved(before,after),
            receipt_sources(original,before,first),receipt_sources(before,after,second),
        ensures receipt_sources(original,after,first+second),
    {
        assert forall|i:int| original.len() <= i < after.len() implies receipt_source(after[i],first+second) by {
            if i<before.len() {
                let j=choose|j:int| 0 <= j < first.len() && crate::json_specs::content(before[i])==crate::json_specs::content(first[j]);
                assert(after[i]==before[i]);assert((first+second)[j]==first[j]);
                assert(receipt_source(after[i],first+second));
            } else {
                let j=choose|j:int| 0 <= j < second.len() && crate::json_specs::content(after[i])==crate::json_specs::content(second[j]);
                assert((first+second)[first.len()+j]==second[j]);
                assert(receipt_source(after[i],first+second));
            }
        }
    }
    pub open spec fn roots_covered(receipts:Seq<Value>,roots:Seq<Value>)->bool {
        forall|i:int| 0 <= i < roots.len() ==> value_covered(receipts,roots[i])
    }
    pub proof fn receipt_coverage_join(before:Seq<Value>,after:Seq<Value>,first:Seq<Value>,second:Seq<Value>)
        requires receipt_preserved(before,after),roots_covered(before,first),roots_covered(after,second),
        ensures roots_covered(after,first+second),
    {
        assert forall|i:int| 0 <= i < (first+second).len() implies value_covered(after,(first+second)[i]) by {
            if i<first.len() {
                let j=choose|j:int| 0 <= j < before.len() && (forall|depth:nat| crate::json_specs::observe(#[trigger] before[j],depth)==crate::json_specs::observe(first[i],depth));
                assert(after[j]==before[j]);
                assert(value_covered(after,(first+second)[i]));
            } else {
                assert((first+second)[i]==second[i-first.len()]);
            }
        }
    }
    pub proof fn plan_prefix(operations:Seq<Operation>,count:int,recover:bool)
        requires 0 <= count <= operations.len(),
        ensures action_prefix(handles(operations.take(count),recover),handles(operations,recover)),
            action_prefix(releases(operations.take(count)),releases(operations)),
        decreases operations.len()-count,
    {
        if count<operations.len() {
            plan_prefix(operations.drop_last(),count,recover);
            assert(operations.drop_last().take(count) =~= operations.take(count));
            action_prefix_extend(handles(operations.take(count),recover),handles(operations.drop_last(),recover),handle_actions(operations.last(),recover));
            action_prefix_extend(releases(operations.take(count)),releases(operations.drop_last()),release_actions(operations.last()));
        } else {
            assert(operations.take(count) =~= operations);
        }
    }
}
#[cfg(not(straw_verus))]
pub trait RecoveryModel {}
#[cfg(not(straw_verus))]
impl<T> RecoveryModel for T {}

/// Each leaf is one named native action; selection of the exact registered
/// roots, key, owner and settlement IDs remains in the checked implementation.
/// An error may have completed that leaf, so retry starts from fresh replay.
#[cfg_attr(straw_verus, verus_verify)]
pub trait RecoveryIo: RecoveryModel {
    type Error;
    #[cfg_attr(straw_verus,verus_spec(result=>ensures completed(old(self)@,final(self)@,RecoveryAction::Validate(roots@),result.is_ok())))]
    fn validate(&mut self, roots: &[Value]) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>ensures completed(old(self)@,final(self)@,RecoveryAction::Stage(roots@),result.is_ok())))]
    fn stage(&mut self, roots: &[Value]) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>ensures completed(old(self)@,final(self)@,RecoveryAction::Consume(key@,refs@),result.is_ok())))]
    fn consume(&mut self, key: &str, refs: &[Value]) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>ensures completed(old(self)@,final(self)@,RecoveryAction::Release(owner@),result.is_ok())))]
    fn release(&mut self, owner: &str) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>ensures completed(old(self)@,final(self)@,RecoveryAction::Commit(ids@,roots@),result.is_ok())))]
    fn commit(&mut self, ids: &[u64], roots: &[Value]) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(ensures final(self)@==old(self)@))]
    fn checkpoint(&mut self) -> Result<(), Self::Error>;
}
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures result.is_ok() ==> final(io)@==old(io)@+handle_actions(*operation,recover),
        action_prefix(final(io)@,old(io)@+handle_actions(*operation,recover)),
        result.is_ok() ==> roots_covered(final(receipts)@,recovered_roots(*operation,recover)),
        receipt_preserved(old(receipts)@,final(receipts)@),
        receipt_sources(old(receipts)@,final(receipts)@,if recover && operation.results.is_some() {operation.results->Some_0.roots@} else {Seq::empty()}),
))]
#[allow(
    clippy::collapsible_if,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
fn recover_operation<I: RecoveryIo>(
    io: &mut I,
    operation: &Operation,
    recover: bool,
    receipts: &mut Vec<Value>,
) -> Result<(), I::Error> {
    if recover {
        if let Some(results) = &operation.results {
            io.validate(&results.roots)?;
            io.stage(&results.roots)?;
            keep_results(receipts, &results.roots);
        }
    }
    Ok(())
}
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures result.is_ok() ==> final(io)@==old(io)@+release_actions(*operation),
        action_prefix(final(io)@,old(io)@+release_actions(*operation)),
))]
#[allow(
    clippy::collapsible_if,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
fn release_operation<I: RecoveryIo>(io: &mut I, operation: &Operation) -> Result<(), I::Error> {
    if let Some(results) = &operation.results {
        if !results.intermediates.is_empty() {
            io.consume(&operation.evidence.key, &results.intermediates)?;
        }
    }
    if let Some(owner) = &operation.evidence.owner {
        io.release(owner)?;
    }
    io.checkpoint()
}
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures result.is_ok() ==> final(io)@ == old(io)@+handles(operations@,recover)+releases(operations@)
        +(if operations.len()>0 {seq![RecoveryAction::Commit(sequences(operations@),final(receipts)@)]} else {Seq::empty()}),
        action_prefix(final(io)@,old(io)@+handles(operations@,recover)+releases(operations@)
            +(if operations.len()>0 {seq![RecoveryAction::Commit(sequences(operations@),final(receipts)@)]} else {Seq::empty()})),
        receipt_preserved(old(receipts)@,final(receipts)@),
        result.is_ok() ==> receipt_sources(old(receipts)@,final(receipts)@,recovery_roots(operations@,recover)),
        result.is_ok() ==> roots_covered(final(receipts)@,recovery_roots(operations@,recover)),
))]
#[allow(
    clippy::question_mark,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn reconcile_registered<I: RecoveryIo>(
    io: &mut I,
    operations: &[Operation],
    recover: bool,
    receipts: &mut Vec<Value>,
) -> Result<(), I::Error> {
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=operations.len(),io@==old(io)@+handles(operations@.take(index as int),recover),
            receipt_preserved(old(receipts)@,receipts@),
            receipt_sources(old(receipts)@,receipts@,recovery_roots(operations@.take(index as int),recover)),
            roots_covered(receipts@,recovery_roots(operations@.take(index as int),recover)),
        decreases operations.len()-index,
    ))]
    while index < operations.len() {
        #[cfg(straw_verus)]
        let before_receipts = verus_exec_expr!(Ghost(receipts@));
        if let Err(error) = recover_operation(io, &operations[index], recover, receipts) {
            #[cfg(straw_verus)]
            proof! {
                plan_extend(operations@,index as int,recover);
                plan_prefix(operations@,index as int+1,recover);
                action_prefix_context(old(io)@,handles(operations@.take(index as int+1),recover),handles(operations@,recover));
                action_prefix_transitive(io@,old(io)@+handles(operations@.take(index as int+1),recover),old(io)@+handles(operations@,recover));
                action_prefix_extend(io@,old(io)@+handles(operations@,recover),releases(operations@)+seq![RecoveryAction::Commit(sequences(operations@),receipts@)]);
            }
            return Err(error);
        }
        #[cfg(straw_verus)]
        proof! {
            plan_extend(operations@,index as int,recover);
            receipt_sources_join(old(receipts)@,before_receipts@,receipts@,recovery_roots(operations@.take(index as int),recover),recovered_roots(operations@[index as int],recover));
            receipt_coverage_join(before_receipts@,receipts@,recovery_roots(operations@.take(index as int),recover),recovered_roots(operations@[index as int],recover));
        }
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(operations@.take(index as int) =~= operations@);}
    index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=operations.len(),io@==old(io)@+handles(operations@,recover)+releases(operations@.take(index as int)),
            receipt_preserved(old(receipts)@,receipts@),
            receipt_sources(old(receipts)@,receipts@,recovery_roots(operations@,recover)),
            roots_covered(receipts@,recovery_roots(operations@,recover)),
        decreases operations.len()-index,
    ))]
    while index < operations.len() {
        if let Err(error) = release_operation(io, &operations[index]) {
            #[cfg(straw_verus)]
            proof! {
                plan_extend(operations@,index as int,recover);
                plan_prefix(operations@,index as int+1,recover);
                action_prefix_context(old(io)@+handles(operations@,recover),releases(operations@.take(index as int+1)),releases(operations@));
                action_prefix_transitive(io@,old(io)@+handles(operations@,recover)+releases(operations@.take(index as int+1)),old(io)@+handles(operations@,recover)+releases(operations@));
                action_prefix_extend(io@,old(io)@+handles(operations@,recover)+releases(operations@),seq![RecoveryAction::Commit(sequences(operations@),receipts@)]);
            }
            return Err(error);
        }
        #[cfg(straw_verus)]
        proof! {plan_extend(operations@,index as int,recover);}
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(operations@.take(index as int) =~= operations@);}
    if !operations.is_empty() {
        let mut ids = Vec::new();
        index = 0;
        #[cfg_attr(straw_verus,verus_spec(
            invariant index<=operations.len(),ids.len()==index,
                forall|i:int| 0 <= i < index ==> ids@[i]==operations@[i].sequence,
                io@==old(io)@+handles(operations@,recover)+releases(operations@),
                receipt_preserved(old(receipts)@,receipts@),
                receipt_sources(old(receipts)@,receipts@,recovery_roots(operations@,recover)),
                roots_covered(receipts@,recovery_roots(operations@,recover)),
            decreases operations.len()-index,
        ))]
        while index < operations.len() {
            ids.push(operations[index].sequence);
            index += 1;
        }
        #[cfg(straw_verus)]
        proof! {assert(ids@ =~= sequences(operations@));}
        io.commit(&ids, receipts)?;
    }
    Ok(())
}
