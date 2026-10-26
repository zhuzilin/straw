//! Production preparation-owner coverage and exact writer-directory sealing.
use crate::writer_registry::Operation;
use serde_json::Value;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub struct Held {
    pub owner: String,
    pub paths: Vec<String>,
}
#[cfg(straw_verus)]
verus! {
    pub trait CoverageModel: View<V=Set<Seq<char>>> {
        type Context;
        spec fn context(&self)->Self::Context;
        spec fn certifies(context:Self::Context,dependency:Value,path:Seq<char>)->bool;
    }
}
#[cfg(not(straw_verus))]
pub trait CoverageModel {}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Coverage: CoverageModel {
    #[cfg_attr(straw_verus,verus_spec(ensures final(self).context()==old(self).context(),final(self)@==Set::<Seq<char>>::empty()))]
    fn reset(&mut self);
    #[cfg_attr(straw_verus,verus_spec(
        ensures final(self).context()==old(self).context(),old(self)@.subset_of(final(self)@),
            forall|path:Seq<char>| final(self)@.contains(path) && !old(self)@.contains(path) ==> Self::certifies(old(self).context(),*dependency,path),
    ))]
    fn include(&mut self, dependency: &Value);
    #[cfg_attr(straw_verus,verus_spec(result=>ensures result==self@.contains(path@)))]
    fn contains(&self, path: &str) -> bool;
}
#[cfg(straw_verus)]
verus! {
    pub open spec fn registered(operations:Seq<Operation>,owner:Seq<char>)->bool {
        exists|i:int| 0 <= i < operations.len() && crate::writer_retirement::named(operations[i].evidence,owner,true)
    }
    pub open spec fn certified<C:Coverage>(context:C::Context,operations:Seq<Operation>,owner:Seq<char>,path:Seq<char>)->bool {
        exists|i:int,j:int| 0 <= i < operations.len() && crate::writer_retirement::named(operations[i].evidence,owner,true)
            && 0 <= j < operations[i].evidence.dependencies.len()
            && C::certifies(context,operations[i].evidence.dependencies@[j],path)
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures result==registered(operations@,owner@)))]
fn registered_owner(operations: &[Operation], owner: &str) -> bool {
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=operations.len(),forall|i:int| 0 <= i < index ==> !crate::writer_retirement::named(operations@[i].evidence,owner@,true),
        decreases operations.len()-index,
    ))]
    while index < operations.len() {
        if crate::writer_retirement::named_operation(&operations[index].evidence, owner, true) {
            return true;
        }
        index += 1;
    }
    false
}
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures final(io).context()==old(io).context(),result ==> forall|i:int| 0 <= i < paths.len() ==> certified::<C>(old(io).context(),operations@,owner@,paths@[i]@),
))]
fn certified_owner<C: Coverage>(
    io: &mut C,
    operations: &[Operation],
    owner: &str,
    paths: &[String],
) -> bool {
    io.reset();
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=operations.len(),io.context()==old(io).context(),
            forall|path:Seq<char>| io@.contains(path) ==> certified::<C>(old(io).context(),operations@,owner@,path),
        decreases operations.len()-index,
    ))]
    while index < operations.len() {
        if crate::writer_retirement::named_operation(&operations[index].evidence, owner, true) {
            let dependencies = &operations[index].evidence.dependencies;
            let mut dependency = 0;
            #[cfg_attr(straw_verus,verus_spec(
                invariant index<operations.len(),dependency<=dependencies.len(),io.context()==old(io).context(),
                    dependencies@==operations@[index as int].evidence.dependencies@,
                    crate::writer_retirement::named(operations@[index as int].evidence,owner@,true),
                    forall|path:Seq<char>| io@.contains(path) ==> certified::<C>(old(io).context(),operations@,owner@,path),
                decreases dependencies.len()-dependency,
            ))]
            while dependency < dependencies.len() {
                #[cfg(straw_verus)]
                let before = verus_exec_expr!(Ghost(io@));
                io.include(&dependencies[dependency]);
                #[cfg(straw_verus)]
                proof! {
                    assert forall|path:Seq<char>| io@.contains(path) implies certified::<C>(old(io).context(),operations@,owner@,path) by {
                        if !before@.contains(path) {
                            assert(C::certifies(io.context(),dependencies@[dependency as int],path));
                            assert(certified::<C>(old(io).context(),operations@,owner@,path));
                        }
                    }
                }
                dependency += 1;
            }
        }
        index += 1;
    }
    let mut path = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant path<=paths.len(),io.context()==old(io).context(),
            forall|value:Seq<char>| io@.contains(value) ==> certified::<C>(old(io).context(),operations@,owner@,value),
            forall|i:int| 0 <= i < path ==> certified::<C>(old(io).context(),operations@,owner@,paths@[i]@),
        decreases paths.len()-path,
    ))]
    while path < paths.len() {
        if !io.contains(&paths[path]) {
            return false;
        }
        path += 1;
    }
    true
}
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures result==(prefix.len()<=value.len() && forall|i:int| 0 <= i < prefix.len() ==> value@[i]==prefix@[i]),
))]
pub(crate) fn starts_with(value: &[u8], prefix: &[u8]) -> bool {
    if value.len() < prefix.len() {
        return false;
    }
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=prefix.len(),prefix.len()<=value.len(),forall|i:int| 0 <= i < index ==> value@[i]==prefix@[i],
        decreases prefix.len()-index,
    ))]
    while index < prefix.len() {
        if value[index] != prefix[index] {
            return false;
        }
        index += 1;
    }
    true
}
#[cfg_attr(straw_verus, verus_verify)]
pub enum Failure {
    Unregistered,
    Uncertified,
}
#[cfg(straw_verus)]
verus! {
    pub open spec fn private_owner(owner:Seq<u8>,prefix:Seq<u8>)->bool {
        prefix.len()<=owner.len() && forall|i:int| 0 <= i < prefix.len() ==> owner[i]==prefix[i]
    }

}
// Prefix matching uses UTF-8 bytes. The loop's public contract uses the same
// actual String byte view, rather than pretending arbitrary Unicode is ASCII.
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures final(io).context()==old(io).context(),
        result.is_ok() ==> forall|i:int| 0 <= i < owners.len() && private_owner(vstd::utf8::encode_utf8(owners@[i].owner@),prefix@)
            ==> registered(operations@,owners@[i].owner@)
                && forall|j:int| 0 <= j < owners@[i].paths.len() ==> certified::<C>(old(io).context(),operations@,owners@[i].owner@,owners@[i].paths@[j]@),
))]
#[allow(
    clippy::redundant_as_str,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn validate_owners<C: Coverage>(
    io: &mut C,
    operations: &[Operation],
    owners: &[Held],
    prefix: &[u8],
) -> Result<(), Failure> {
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=owners.len(),io.context()==old(io).context(),
            forall|i:int| 0 <= i < index && private_owner(vstd::utf8::encode_utf8(owners@[i].owner@),prefix@)
                ==> registered(operations@,owners@[i].owner@)
                    && forall|j:int| 0 <= j < owners@[i].paths.len() ==> certified::<C>(old(io).context(),operations@,owners@[i].owner@,owners@[i].paths@[j]@),
        decreases owners.len()-index,
    ))]
    while index < owners.len() {
        let held = &owners[index];
        if starts_with(held.owner.as_str().as_bytes(), prefix) {
            if !registered_owner(operations, &held.owner) {
                return Err(Failure::Unregistered);
            }
            if !certified_owner(io, operations, &held.owner, &held.paths) {
                return Err(Failure::Uncertified);
            }
        }
        index += 1;
    }
    Ok(())
}

#[cfg(straw_verus)]
verus! {
    pub trait ParentModel {
        type Context;
        type Identity;
        spec fn name(identity:Self::Identity)->Seq<u8>;
        spec fn context(&self)->Self::Context;
        spec fn parent(context:Self::Context,path:Seq<char>)->Option<Seq<u8>>;
    }
}
#[cfg(not(straw_verus))]
pub trait ParentModel {
    type Identity;
}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Parents: ParentModel {
    type Error;
    #[cfg_attr(straw_verus,verus_spec(result=>
        ensures final(self).context()==old(self).context(),
            result.is_ok() ==> match result->Ok_0 {None=>Self::parent(old(self).context(),path@).is_none(),Some(parent)=>Self::parent(old(self).context(),path@)==Some(Self::name(parent))},
    ))]
    fn canonical_parent(&mut self, path: &str) -> Result<Option<Self::Identity>, Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>ensures result==(Self::name(*left)==Self::name(*right))))]
    fn same_parent(&self, left: &Self::Identity, right: &Self::Identity) -> bool;
}
#[cfg(straw_verus)]
verus! {
    pub open spec fn belongs<P:Parents>(context:P::Context,path:(String,bool),writer:Seq<u8>)->bool {
        !path.1 && P::parent(context,path.0@)==Some(writer)
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures final(io).context()==old(io).context(),
        result.is_ok() ==> forall|i:int| 0 <= i < result->Ok_0.len() ==> 0 <= result->Ok_0@[i] < packs.len()
            && belongs::<P>(old(io).context(),packs@[result->Ok_0@[i] as int],P::name(*writer)),
        result.is_ok() ==> forall|i:int| 0 <= i < packs.len() && belongs::<P>(old(io).context(),packs@[i],P::name(*writer)) ==> result->Ok_0@.contains(i as usize),
        result.is_ok() ==> forall|i:int,j:int| 0 <= i < j < result->Ok_0.len() ==> result->Ok_0@[i]<result->Ok_0@[j],
))]
#[allow(
    clippy::collapsible_if,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn seal_candidates<P: Parents>(
    io: &mut P,
    packs: &[(String, bool)],
    writer: &P::Identity,
) -> Result<Vec<usize>, P::Error> {
    let mut selected = Vec::new();
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=packs.len(),io.context()==old(io).context(),
            forall|i:int| 0 <= i < selected.len() ==> 0 <= selected@[i] < index && belongs::<P>(old(io).context(),packs@[selected@[i] as int],P::name(*writer)),
            forall|i:int| 0 <= i < index && belongs::<P>(old(io).context(),packs@[i],P::name(*writer)) ==> selected@.contains(i as usize),
            forall|i:int,j:int| 0 <= i < j < selected.len() ==> selected@[i]<selected@[j],
        decreases packs.len()-index,
    ))]
    while index < packs.len() {
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(selected@));
        if !packs[index].1 {
            let parent = io.canonical_parent(&packs[index].0)?;
            if let Some(parent) = parent {
                if io.same_parent(&parent, writer) {
                    selected.push(index);
                }
            }
        }
        #[cfg(straw_verus)]
        proof! {
            assert forall|i:int| 0 <= i < index+1 && belongs::<P>(old(io).context(),packs@[i],P::name(*writer)) implies selected@.contains(i as usize) by {
                if i<index {
                    let position=choose|position:int| 0 <= position < before@.len() && before@[position]==i as usize;
                    assert(selected@[position]==before@[position]);
                } else {
                    assert(selected@.last()==index);
                }
            }
        }
        index += 1;
    }
    Ok(selected)
}

#[cfg_attr(straw_verus,verus_spec(result=>
    ensures result==(forall|i:int| 0 <= i < dependencies.len() ==> crate::closure_node::reference_kind(dependencies@[i])==crate::closure_node::Kind::Manifest),
))]
pub fn result_dependencies(dependencies: &[Value]) -> bool {
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=dependencies.len(),forall|i:int| 0 <= i < index ==> crate::closure_node::reference_kind(dependencies@[i])==crate::closure_node::Kind::Manifest,
        decreases dependencies.len()-index,
    ))]
    while index < dependencies.len() {
        if !matches!(
            crate::closure_node::kind(&dependencies[index]),
            crate::closure_node::Kind::Manifest
        ) {
            return false;
        }
        index += 1;
    }
    true
}
#[cfg_attr(straw_verus, verus_verify)]
pub enum AuthenticationFailure<R, D> {
    Read(R),
    Digest(D),
}
#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::closure::valid(old(source)@),
        forall|value:Value| (old(source)@.key)(value)==(old(digester)@)(crate::json_specs::content(value)),
        forall|i:int| 0 <= i < roots.len() ==> old(source)@.nodes.contains_key((old(source)@.key)(roots@[i])),
        crate::environment::string_hash_contract(),crate::environment::string_order_contract(),
    ensures final(source)@==old(source)@,final(digester)@==old(digester)@,
        result.is_ok() ==> result->Ok_0.len()==roots.len()
            && forall|i:int| 0 <= i < roots.len() ==> result->Ok_0@[i]@==crate::closure::packs(old(source)@,(old(source)@.key)(roots@[i])),
))]
#[allow(
    clippy::type_complexity,
    reason = "Keep component error types visible at the checked I/O boundary."
)]
pub fn authenticate_all<
    S: crate::closure::Source<Reference = Value>,
    D: crate::closure_node::Digest,
>(
    source: &mut S,
    digester: &mut D,
    roots: &[Value],
) -> Result<Vec<std::collections::BTreeSet<String>>, AuthenticationFailure<S::Error, D::Error>> {
    let mut cache = std::collections::HashMap::new();
    let mut authenticated: Vec<std::collections::BTreeSet<String>> = Vec::new();
    let mut index = 0;
    #[cfg(straw_verus)]
    proof! {crate::closure::empty_cache(source@);}
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=roots.len(),authenticated.len()==index,source@==old(source)@,digester@==old(digester)@,
            crate::closure::valid(source@),crate::closure::cached(source@,cache@),
            forall|value:Value| (source@.key)(value)==(digester@)(crate::json_specs::content(value)),
            forall|i:int| 0 <= i < roots.len() ==> source@.nodes.contains_key((source@.key)(roots@[i])),
            forall|i:int| 0 <= i < index ==> authenticated@[i]@==crate::closure::packs(source@,(source@.key)(roots@[i])),
            crate::environment::string_hash_contract(),crate::environment::string_order_contract(),
        decreases roots.len()-index,
    ))]
    while index < roots.len() {
        let key = match digester.digest(&roots[index]) {
            Ok(key) => key,
            Err(error) => return Err(AuthenticationFailure::Digest(error)),
        };
        let root = roots[index].clone();
        let visited = match crate::closure::collect(source, &mut cache, key, root) {
            Ok(visited) => visited,
            Err(error) => return Err(AuthenticationFailure::Read(error)),
        };
        authenticated.push(visited);
        index += 1;
    }
    Ok(authenticated)
}
