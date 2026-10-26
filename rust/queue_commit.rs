//! The actual owned WAL append and guarded business-application chain.
use crate::file_adapter::{Library, NativeFile};
use crate::journal_owner::{Borrowed, Journal};
use crate::{queue_application, queue_history};
use serde_json::Value;
use std::collections::HashMap;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub(crate) enum Error<E> {
    Append(crate::journal_owner::Error<E>),
    Apply(queue_application::Error<E>),
}

#[cfg_attr(straw_verus, verus_verify)]
pub(crate) struct Requests {
    pub roots: Vec<(String, Vec<Value>)>,
    pub references: Vec<Value>,
    #[cfg(straw_verus)]
    pub(crate) entries: Ghost<Seq<(String, Value)>>,
}

#[cfg(straw_verus)]
verus! {
    pub(crate) open spec fn business_result<E>(before:queue_history::Business,after:queue_history::Business,
        events:Value,result:Result<(),Error<E>>) -> bool {
        match result {
            Ok(()) | Err(Error::Apply(queue_application::Error::After(_))) =>
                queue_history::transaction_result(before,after,events,Ok(())),
            Err(Error::Append(_)) | Err(Error::Apply(queue_application::Error::Before(_))) => after==before,
            Err(Error::Apply(queue_application::Error::Body(error))) =>
                queue_history::transaction_result(before,after,events,Err(error)),
        }
    }
    pub(crate) open spec fn handoff_after(before:Seq<(String,Vec<Value>)>,handoff:Option<(String,Vec<Value>)>)->Seq<(String,Vec<Value>)> {
        match handoff {None=>before,Some(value)=>before.push(value)}
    }
    pub(crate) open spec fn request_at(root:(String,Vec<Value>),reference:Value,entry:(String,Value),prefix:Seq<char>)->bool {
        root.0@==prefix+entry.0@ && root.1@.len()==1
            && crate::json_specs::content(root.1@[0])==crate::json_specs::content(entry.1) && reference==entry.1
    }
    pub(crate) open spec fn requests_prefix(roots:Seq<(String,Vec<Value>)>,refs:Seq<Value>,entries:Seq<(String,Value)>,prefix:Seq<char>)->bool {
        roots.len()==refs.len() && roots.len()<=entries.len()
            && forall|i:int| 0<=i<roots.len() ==> request_at(roots[i],refs[i],entries[i],prefix)
    }
    pub(crate) open spec fn unique_owners(roots:Seq<(String,Vec<Value>)>)->bool {
        forall|i:int,j:int| 0<=i<j<roots.len() ==> roots[i].0@ != roots[j].0@
    }
    pub(crate) open spec fn reactivated_prefix(output:Seq<(String,Value)>,entries:Seq<(String,Value)>,prefix:Seq<char>)->bool {
        output.len()<=entries.len() && forall|i:int| 0<=i<output.len()
            ==> output[i].0@==prefix+entries[i].0@ && output[i].1==entries[i].1
    }
    pub(crate) open spec fn reactivated_covers(output:Seq<(String,Value)>,input:Map<String,Value>,prefix:Seq<char>)->bool {
        exists|entries:Seq<(String,Value)>| entries.to_set()==input.kv_pairs() && entries.no_duplicates()
            && output.len()==entries.len() && reactivated_prefix(output,entries,prefix)
    }
    pub(crate) proof fn prefixed_owners_unique<V,T>(input:Map<String,V>,entries:Seq<(String,V)>,
        roots:Seq<(String,T)>,prefix:Seq<char>)
        requires crate::environment::string_content_contract(),entries.to_set()==input.kv_pairs(),entries.no_duplicates(),
            roots.len()<=entries.len(),forall|i:int| 0<=i<roots.len() ==> roots[i].0@==prefix+entries[i].0@,
        ensures forall|i:int,j:int| 0<=i<j<roots.len() ==> roots[i].0@ != roots[j].0@,
    {
        assert forall|i:int,j:int| 0<=i<j<roots.len() implies roots[i].0@ != roots[j].0@ by {
            if roots[i].0@==roots[j].0@ {
                let a=entries[i].0;let b=entries[j].0;
                assert((prefix+a@).skip(prefix.len() as int) =~= a@);
                assert((prefix+b@).skip(prefix.len() as int) =~= b@);
                assert(a@==b@);assert(a==b);
                assert(entries.to_set().contains(entries[i]));assert(entries.to_set().contains(entries[j]));
                assert(input.contains_key(a));assert(entries[i].1==input[a]);assert(entries[j].1==input[b]);
                assert(entries[i]==entries[j]);assert(false);
            }
        }
    }
    impl Requests {
        pub(crate) closed spec fn covers(&self,input:Map<String,Value>,prefix:Seq<char>)->bool {
            self.entries@.to_set()==input.kv_pairs() && self.entries@.no_duplicates()
                && self.roots@.len()==self.entries@.len()
                && requests_prefix(self.roots@,self.references@,self.entries@,prefix)
        }
    }
    pub(crate) open spec fn training_event(value:Value)->bool {
        let kind=crate::json_specs::text(crate::queue_roots::field_or_null(value,"type"@));
        (kind==Some("ConsumerState"@) || kind==Some("BatchReady"@))
            && crate::json_specs::text(crate::queue_roots::field_or_null(value,"consumer_id"@))==Some("training"@)
    }
    pub(crate) open spec fn revived(retired:Seq<Value>,prefix:u64,positions:Seq<Value>)->bool {
        exists|i:int| 0<=i<retired.len() && match crate::json_specs::json(#[trigger] retired[i]) {
            crate::json_specs::Json::Number(Some(p))=>p>=prefix && !crate::queue_roots::finished(positions,retired[i]),
            _=>false,
        }
    }
}

#[cfg_attr(straw_verus,verus_spec(result =>
    requires crate::environment::string_hash_contract(),crate::environment::string_content_contract(),
    ensures reactivated_covers(result@,input@,prefix@),
        forall|i:int,j:int| 0<=i<j<result@.len() ==> result@[i].0@ != result@[j].0@,
))]
pub(crate) fn reactivation_requests(
    prefix: &str,
    input: HashMap<String, Value>,
) -> Vec<(String, Value)> {
    let mut it = crate::catalog_decode::owned_entries(input);
    #[cfg(straw_verus)]
    let entries = verus_exec_expr!(Ghost(crate::collection_specs::remaining(it)));
    let mut output = Vec::new();
    #[cfg_attr(straw_verus,verus_spec(
        invariant crate::environment::string_content_contract(),entries@.to_set()==input@.kv_pairs(),entries@.no_duplicates(),
            reactivated_prefix(output@,entries@,prefix@),crate::collection_specs::remaining(it)==entries@.skip(output@.len() as int),
        decreases crate::collection_specs::remaining(it).len(),
    ))]
    loop {
        let Some((key, reference)) = it.next() else {
            #[cfg(straw_verus)]
            proof! {
                prefixed_owners_unique(input@,entries@,output@,prefix@);
                assert(exists|items:Seq<(String,Value)>| items==entries@ && items.to_set()==input@.kv_pairs() && items.no_duplicates()
                    && output@.len()==items.len() && reactivated_prefix(output@,items,prefix@));
            }
            return output;
        };
        #[cfg(straw_verus)]
        proof! {assert((key,reference)==entries@[output@.len() as int]);}
        let mut owner = prefix.to_owned();
        owner.push_str(&key);
        output.push((owner, reference));
    }
}

#[cfg_attr(straw_verus,verus_spec(result =>
    ensures match result {Some(i)=>i<events.len() && training_event(events@[i as int])
            && forall|j:int| 0<=j<i ==> !training_event(events@[j]),
        None=>forall|j:int| 0<=j<events.len() ==> !training_event(events@[j])},
))]
pub(crate) fn restore_event(events: &[Value]) -> Option<usize> {
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=events.len(),forall|i:int| 0<=i<index ==> !training_event(events@[i]),
        decreases events.len()-index,
    ))]
    while index < events.len() {
        let event = &events[index];
        let kind = crate::queue_roots::field_value(event, "type").as_str();
        if (kind == Some("ConsumerState") || kind == Some("BatchReady"))
            && crate::queue_roots::field_value(event, "consumer_id").as_str() == Some("training")
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

#[cfg_attr(straw_verus,verus_spec(result =>
    ensures result == (prefix<retired_prefix || revived(retired@,prefix,positions@)),
))]
#[allow(
    clippy::collapsible_if,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub(crate) fn retention_needed(
    retired_prefix: u64,
    retired: &[Value],
    prefix: u64,
    positions: &[Value],
) -> bool {
    if prefix < retired_prefix {
        return true;
    }
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=retired.len(),prefix>=retired_prefix,!revived(retired@.take(index as int),prefix,positions@),
        decreases retired.len()-index,
    ))]
    while index < retired.len() {
        if let Some(position) = retired[index].as_u64() {
            if position >= prefix && !crate::queue_roots::contains(positions, &retired[index]) {
                #[cfg(straw_verus)]
                proof! {assert(revived(retired@,prefix,positions@));}
                return true;
            }
        }
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(retired@.take(index as int) =~= retired@);}
    false
}

/// Build each retained owner and its eventual staging root from one traversal.
#[cfg_attr(straw_verus,verus_spec(result =>
    requires crate::environment::string_hash_contract(),crate::environment::string_content_contract(),
    ensures result.covers(references@,prefix@),unique_owners(result.roots@),
))]
#[allow(
    clippy::vec_init_then_push,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub(crate) fn owner_requests(prefix: &str, references: HashMap<String, Value>) -> Requests {
    let mut it = crate::catalog_decode::owned_entries(references);
    #[cfg(straw_verus)]
    let entries = verus_exec_expr!(Ghost(crate::collection_specs::remaining(it)));
    let mut roots = Vec::new();
    let mut refs = Vec::new();
    #[cfg_attr(straw_verus,verus_spec(
        invariant crate::environment::string_content_contract(),entries@.to_set()==references@.kv_pairs(),entries@.no_duplicates(),
            requests_prefix(roots@,refs@,entries@,prefix@),
            crate::collection_specs::remaining(it)==entries@.skip(roots@.len() as int),
        decreases crate::collection_specs::remaining(it).len(),
    ))]
    loop {
        let Some((key, reference)) = it.next() else {
            #[cfg(straw_verus)]
            proof! {
                assert forall|i:int| 0<=i<roots@.len() implies roots@[i].0@==prefix@+entries@[i].0@ by {
                    assert(request_at(roots@[i],refs@[i],entries@[i],prefix@));
                }
                prefixed_owners_unique(references@,entries@,roots@,prefix@);
            }
            return Requests {
                roots,
                references: refs,
                #[cfg(straw_verus)]
                entries,
            };
        };
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost((roots@,refs@)));
        #[cfg(straw_verus)]
        proof! {assert((key,reference)==entries@[roots@.len() as int]);}
        let mut owner = prefix.to_owned();
        owner.push_str(&key);
        #[cfg(straw_verus)]
        proof! {assert(owner@==prefix@+key@);}
        let mut one = Vec::new();
        one.push(reference.clone());
        roots.push((owner, one));
        refs.push(reference);
        #[cfg(straw_verus)]
        proof! {
            assert(before@.0.len()<entries@.len());
            assert(roots@.last().0@==prefix@+entries@[before@.0.len() as int].0@);
            assert(roots@.last().1@.len()==1);
            assert(crate::json_specs::content(roots@.last().1@[0])==crate::json_specs::content(entries@[before@.0.len() as int].1));
            assert(refs@.last()==entries@[before@.0.len() as int].1);
            assert forall|i:int| 0<=i<roots@.len() implies request_at(roots@[i],refs@[i],entries@[i],prefix@) by {
                if i<before@.0.len() {
                    assert(requests_prefix(before@.0,before@.1,entries@,prefix@));
                    assert(request_at(before@.0[i],before@.1[i],entries@[i],prefix@));
                    assert(roots@[i]==before@.0[i]);
                    assert(refs@[i]==before@.1[i]);
                    assert(before@.0[i].0@==prefix@+entries@[i].0@);
                    assert(before@.0[i].1@.len()==1);
                    assert(crate::json_specs::content(before@.0[i].1@[0])==crate::json_specs::content(entries@[i].1));
                    assert(before@.1[i]==entries@[i].1);
                } else {assert(i==before@.0.len());}
            }
            assert(roots@.len()==refs@.len());
            assert(roots@.len()<=entries@.len());
            assert(requests_prefix(roots@,refs@,entries@,prefix@)) by {reveal(requests_prefix);}
        }
    }
}

/// Retention and the post-application staging receipt surround this function.
/// This function owns every mutable queue field while its callbacks run.
#[cfg_attr(straw_verus,verus_spec(result =>
    requires old(journal).valid(),crate::environment::string_order_contract(),
    ensures final(journal).valid(),final(journal).path==old(journal).path,
        final(journal).file.is_some()==old(journal).file.is_some(),
        final(journal).origin==old(journal).origin,final(journal).created==old(journal).created,
        final(journal).state.read_only==old(journal).state.read_only,
        final(journal).state.transactions@==old(journal).state.transactions@,
        final(journal).state.incomplete_tail==old(journal).state.incomplete_tail,
        business_result((*old(state),*old(epoch),*old(order),*old(sealed)),
            (*final(state),*final(epoch),*final(order),*final(sealed)),*events,result),
        match result {
            Err(Error::Append(error)) => final(pending)@==old(pending)@
                && final(journal).state.sequence==old(journal).state.sequence
                && final(journal).state.offset==old(journal).state.offset
                && (final(journal).state==old(journal).state || final(journal).state.poisoned),
            _ => final(pending)@==handoff_after(old(pending)@,handoff)
                && final(journal).state.sequence as int==old(journal).state.sequence as int+1
                && final(journal).boundary() && !final(journal).state.read_only
                && final(journal).permission@@.bytes.take(old(journal).permission@@.bytes.len() as int)==old(journal).permission@@.bytes
                && crate::journal_append::accepted(old(journal).state,final(journal).state,*events,final(journal).permission@@.bytes),
        },
        result.is_ok() ==> !final(journal).state.poisoned,
        match result {Err(Error::Apply(_))=>final(journal).state.poisoned,_=>true},
))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn persist_and_apply<P, L: Library, C: queue_history::Caches, H>(
    journal: &mut Journal<P>,
    state: &mut Value,
    epoch: &mut String,
    order: &mut usize,
    sealed: &mut bool,
    pending: &mut Vec<(String, Vec<Value>)>,
    handoff: Option<(String, Vec<Value>)>,
    events: &Value,
    caches: &mut C,
    hooks: &mut H,
) -> Result<(), Error<L::Error>>
where
    H: queue_application::Hooks<Error = L::Error>,
    for<'a> H: crate::wal_sink::Hooks<NativeFile<'a, L>>,
{
    if let Err(error) = journal.append_with::<L, _>(events, Borrowed(hooks)) {
        return Err(Error::Append(error));
    }
    if let Some(value) = handoff {
        pending.push(value);
    }
    #[cfg(straw_verus)]
    proof! { journal.owner_facts(); }
    queue_application::arm(&mut journal.state);
    let confirmation = match queue_application::apply(
        &journal.state,
        state,
        epoch,
        order,
        sealed,
        events,
        caches,
        hooks,
    ) {
        Ok(confirmation) => confirmation,
        Err(error) => {
            #[cfg(straw_verus)]
            proof! { journal.owner_facts(); }
            return Err(Error::Apply(error));
        }
    };
    queue_application::confirm(&mut journal.state, confirmation);
    #[cfg(straw_verus)]
    proof! { journal.owner_facts(); }
    Ok(())
}
