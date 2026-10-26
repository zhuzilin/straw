//! The actual staging recipe traversal, including persisted recipes after GC.
use crate::closure_node::{Digest, Kind};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub struct Plan {
    pub nodes: HashSet<String>,
    pub raw: HashSet<String>,
    pub additions: HashMap<String, Vec<String>>,
}
#[cfg_attr(straw_verus, verus_verify)]
pub enum Error<R, D> {
    Read(R),
    Digest(D),
    Reference,
    Missing,
    Dependencies,
    Records,
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn staged(key:String,name:String)->bool { name@=="staged:"@+key@ }
    pub open spec fn ids(values:Seq<Value>,key:spec_fn(Value)->String)->Set<String> {
        values.map(|i:int,v:Value| key(v)).to_set()
    }
    pub open spec fn roots(values:Seq<Value>,key:spec_fn(Value)->String,kind:Kind)->Set<String> {
        ids(values.filter(|v:Value| crate::closure_node::reference_kind(v)==kind),key)
    }
    pub open spec fn agrees(known:Map<String,Vec<String>>,g:crate::closure::Graph<Value>)->bool {
        forall|key:String| known.contains_key(key) ==> g.nodes.contains_key(key)
            && known[key]@.to_set()==g.nodes[key].children
            && known[key]@.to_set().subset_of(known.dom())
    }
    pub open spec fn bound(g:crate::closure::Graph<Value>,known:Map<String,Vec<String>>,pending:Seq<(String,Option<Value>)>)->bool {
        forall|i:int| 0<=i<pending.len() ==> g.nodes.contains_key(pending[i].0)
            && match pending[i].1 {Some(v)=>(g.key)(v)==pending[i].0,None=>known.contains_key(pending[i].0)}
    }
    pub open spec fn reached(g:crate::closure::Graph<Value>,initial:Set<String>)->Set<String> {
        g.nodes.dom().filter(|k:String| exists|root:String| initial.contains(root) && crate::closure::reaches(g,root,k))
    }
    pub proof fn root_reached(g:crate::closure::Graph<Value>,initial:Set<String>,root:String)
        requires initial.contains(root),g.nodes.contains_key(root),
        ensures reached(g,initial).contains(root),
    {crate::closure::root_reaches(g,root);}
    pub proof fn child_reached(g:crate::closure::Graph<Value>,initial:Set<String>,parent:String,child:String)
        requires crate::closure::valid(g),reached(g,initial).contains(parent),g.nodes[parent].children.contains(child),
        ensures reached(g,initial).contains(child),
    {
        let root=choose|root:String| initial.contains(root) && crate::closure::reaches(g,root,parent);
        crate::closure::append_edge(g,root,parent,child);
    }
    pub proof fn roots_push(values:Seq<Value>,key:spec_fn(Value)->String,v:Value,kind:Kind)
        ensures roots(values.push(v),key,kind)==if crate::closure_node::reference_kind(v)==kind {roots(values,key,kind).insert(key(v))} else {roots(values,key,kind)},
    {
        let pred=|v:Value| crate::closure_node::reference_kind(v)==kind;
        values.lemma_filter_push(v,pred);
        if pred(v) {
            let before=values.filter(pred).map(|i:int,v:Value| key(v));
            assert(values.filter(pred).push(v).map(|i:int,v:Value| key(v)) =~= before.push(key(v)));
            before.lemma_push_to_set_commute(key(v));
        }
    }
    pub open spec fn binding(g:crate::closure::Graph<Value>,digester:spec_fn(Seq<u8>)->String)->bool {
        forall|v:Value| staged(digester(crate::json_specs::content(v)),(g.key)(v))
    }
    pub proof fn ids_add(a:Seq<Value>,b:Seq<Value>,key:spec_fn(Value)->String)
        ensures ids(a+b,key)==ids(a,key).union(ids(b,key)),
    {
        let left=a.map(|i:int,v:Value| key(v));let right=b.map(|i:int,v:Value| key(v));
        assert((a+b).map(|i:int,v:Value| key(v)) =~= left+right);
        assert forall|name:String| (left+right).to_set().contains(name) == left.to_set().union(right.to_set()).contains(name) by {
            if (left+right).to_set().contains(name) {
                let i=choose|i:int| 0<=i<(left+right).len() && (left+right)[i]==name;
                if i<left.len() {assert(left[i]==name);} else {assert(right[i-left.len()]==name);}
            } else if left.to_set().contains(name) {
                let i=choose|i:int| 0<=i<left.len() && left[i]==name;assert((left+right)[i]==name);
            } else if right.to_set().contains(name) {
                let i=choose|i:int| 0<=i<right.len() && right[i]==name;assert((left+right)[left.len()+i]==name);
            }
        }
        assert((left+right).to_set() =~= left.to_set().union(right.to_set()));
    }
    pub proof fn ids_member(values:Seq<Value>,key:spec_fn(Value)->String,v:Value)
        requires values.contains(v),ensures ids(values,key).contains(key(v)),
    {
        let i=choose|i:int| 0<=i<values.len() && values[i]==v;
        assert(values.map(|i:int,v:Value| key(v))[i]==key(v));
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Source: crate::closure::SourceModel<Reference = Value> {
    type Error;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires crate::closure::valid(old(self)@),old(self)@.nodes.contains_key((old(self)@.key)(*reference)),
        ensures final(self)@==old(self)@,
            result.is_ok() && crate::closure_node::well_formed(result->Ok_0) ==>
                ids(crate::closure_node::successors(result->Ok_0),old(self)@.key)==old(self)@.nodes[(old(self)@.key)(*reference)].children,
    ))]
    fn manifest(&mut self, reference: &Value) -> Result<Value, Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(ensures final(self)@==old(self)@))]
    fn point(&mut self, name: &str) -> Result<(), Self::Error>;
}

#[cfg_attr(straw_verus,verus_spec(result=>
    ensures final(digester)@==old(digester)@,
        result.is_ok() ==> staged((old(digester)@)(crate::json_specs::content(*value)),result->Ok_0),
))]
pub fn name<D: Digest>(digester: &mut D, value: &Value) -> Result<String, D::Error> {
    let key = digester.digest(value)?;
    let mut result = "staged:".to_owned();
    result.push_str(&key);
    Ok(result)
}

#[cfg_attr(straw_verus,verus_spec(
    requires crate::environment::string_hash_contract(),crate::traversal::valid(old(frontier)),
    ensures crate::traversal::valid(final(frontier)),
        final(frontier).seen@==old(frontier).seen@.union(children@.to_set()),
        crate::closure::keys(final(frontier).pending@)==crate::closure::keys(old(frontier).pending@).union(children@.to_set().difference(old(frontier).seen@)),
        forall|i:int| 0<=i<final(frontier).pending@.len() ==> old(frontier).pending@.contains(final(frontier).pending@[i])
            || (children@.to_set().contains(final(frontier).pending@[i].0) && final(frontier).pending@[i].1.is_none()),
))]
fn known_children(frontier: &mut crate::traversal::Frontier<Option<Value>>, children: &[String]) {
    let mut i = 0usize;
    #[cfg_attr(straw_verus,verus_spec(
        invariant i<=children.len(),crate::environment::string_hash_contract(),crate::traversal::valid(frontier),
            frontier.seen@==old(frontier).seen@.union(children@.take(i as int).to_set()),
            crate::closure::keys(frontier.pending@)==crate::closure::keys(old(frontier).pending@).union(children@.take(i as int).to_set().difference(old(frontier).seen@)),
            forall|j:int| 0<=j<frontier.pending@.len() ==> old(frontier).pending@.contains(frontier.pending@[j])
                || (children@.take(i as int).to_set().contains(frontier.pending@[j].0) && frontier.pending@[j].1.is_none()),
        decreases children.len()-i,
    ))]
    while i < children.len() {
        #[cfg(straw_verus)]
        proof! {crate::closure::keys_push(frontier.pending@,children@[i as int],None);children@.take(i as int).lemma_push_to_set_commute(children@[i as int]);assert(children@.take(i as int+1)==children@.take(i as int).push(children@[i as int]));}
        crate::traversal::schedule(frontier, children[i].clone(), None);
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(children@.take(i as int)==children@);}
}

#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::environment::string_hash_contract(),crate::environment::string_content_contract(),
        crate::traversal::valid(old(frontier)),binding(g@,old(digester)@),
    ensures final(digester)@==old(digester)@,crate::traversal::valid(final(frontier)),
        result.is_ok() ==> final(frontier).seen@==old(frontier).seen@.union(ids(values@,g@.key)),
        result.is_ok() ==> crate::closure::keys(final(frontier).pending@)==crate::closure::keys(old(frontier).pending@).union(ids(values@,g@.key).difference(old(frontier).seen@)),
        result.is_ok() ==> final(children)@.to_set()==old(children)@.to_set().union(ids(values@,g@.key)),
        result.is_ok() ==> forall|i:int| 0<=i<final(frontier).pending@.len() ==> old(frontier).pending@.contains(final(frontier).pending@[i])
            || (ids(values@,g@.key).contains(final(frontier).pending@[i].0) && final(frontier).pending@[i].1.is_some()
                && (g@.key)(final(frontier).pending@[i].1.unwrap())==final(frontier).pending@[i].0),
))]
fn fresh_children<D: Digest>(
    digester: &mut D,
    frontier: &mut crate::traversal::Frontier<Option<Value>>,
    children: &mut Vec<String>,
    values: &[Value],
    #[cfg(straw_verus)] g: Ghost<crate::closure::Graph<Value>>,
) -> Result<(), D::Error> {
    let mut i = 0usize;
    #[cfg_attr(straw_verus,verus_spec(
        invariant i<=values.len(),crate::environment::string_hash_contract(),crate::environment::string_content_contract(),
            crate::traversal::valid(frontier),digester@==old(digester)@,binding(g@,digester@),
            frontier.seen@==old(frontier).seen@.union(ids(values@.take(i as int),g@.key)),
            crate::closure::keys(frontier.pending@)==crate::closure::keys(old(frontier).pending@).union(ids(values@.take(i as int),g@.key).difference(old(frontier).seen@)),
            children@.to_set()==old(children)@.to_set().union(ids(values@.take(i as int),g@.key)),
            forall|j:int| 0<=j<frontier.pending@.len() ==> old(frontier).pending@.contains(frontier.pending@[j])
                || (ids(values@.take(i as int),g@.key).contains(frontier.pending@[j].0) && frontier.pending@[j].1.is_some()
                    && (g@.key)(frontier.pending@[j].1.unwrap())==frontier.pending@[j].0),
        decreases values.len()-i,
    ))]
    while i < values.len() {
        let key = name(digester, &values[i])?;
        #[cfg(straw_verus)]
        proof! {
            assert(key==(g@.key)(values@[i as int]));
            let before=values@.take(i as int).map(|j:int,v:Value| (g@.key)(v));
            assert(values@.take(i as int+1).map(|j:int,v:Value| (g@.key)(v)) =~= before.push(key));
            before.lemma_push_to_set_commute(key);children@.lemma_push_to_set_commute(key);
        }
        children.push(key.clone());
        let value = Some(values[i].clone());
        #[cfg(straw_verus)]
        proof! {crate::closure::keys_push(frontier.pending@,key,value);}
        crate::traversal::schedule(frontier, key, value);
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(values@.take(i as int)==values@);}
    Ok(())
}

#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::environment::string_hash_contract(),crate::environment::string_content_contract(),
        crate::closure::valid(old(source)@),agrees(known@,old(source)@),binding(old(source)@,old(digester)@),
        crate::traversal::valid(old(frontier)),bound(old(source)@,known@,old(frontier).pending@),
        old(source)@.nodes.contains_key(*key),
        reference.is_some() ==> (old(source)@.key)(reference.unwrap())==*key,
        forall|v:Value| crate::closure_node::reference_kind(v)==Kind::Record && old(source)@.nodes.contains_key((old(source)@.key)(v))
            ==> old(source)@.nodes[(old(source)@.key)(v)].children==Set::<String>::empty(),
    ensures final(source)@==old(source)@,final(digester)@==old(digester)@,
        result.is_ok() ==> crate::traversal::valid(final(frontier)) && bound(old(source)@,known@,final(frontier).pending@),
        result.is_ok() ==> final(frontier).seen@==old(frontier).seen@.union(old(source)@.nodes[*key].children),
        result.is_ok() ==> crate::closure::keys(final(frontier).pending@)==crate::closure::keys(old(frontier).pending@).union(old(source)@.nodes[*key].children.difference(old(frontier).seen@)),
        result.is_ok() ==> result->Ok_0.is_some()==!known@.contains_key(*key),
        result.is_ok() && result->Ok_0.is_some() ==> result->Ok_0.unwrap()@.to_set()==old(source)@.nodes[*key].children,
))]
// Keep the reader and digest failures distinct in the checked traversal contract.
#[allow(clippy::type_complexity)]
fn expand<S: Source, D: Digest>(
    source: &mut S,
    digester: &mut D,
    known: &HashMap<String, Vec<String>>,
    frontier: &mut crate::traversal::Frontier<Option<Value>>,
    key: &String,
    reference: Option<Value>,
) -> Result<Option<Vec<String>>, Error<S::Error, D::Error>> {
    if let Some(children) = known.get(key) {
        known_children(frontier, children);
        #[cfg(straw_verus)]
        proof! {
            assert forall|i:int| 0<=i<frontier.pending@.len() implies source@.nodes.contains_key(frontier.pending@[i].0)
                && match frontier.pending@[i].1 {Some(v)=>(source@.key)(v)==frontier.pending@[i].0,None=>known@.contains_key(frontier.pending@[i].0)} by {
                if old(frontier).pending@.contains(frontier.pending@[i]) {
                    let j=choose|j:int| 0<=j<old(frontier).pending@.len() && old(frontier).pending@[j]==frontier.pending@[i];
                }
            }
            assert forall|k:String| crate::closure::keys(frontier.pending@).contains(k) implies crate::closure::keys(old(frontier).pending@).union(source@.nodes[*key].children).contains(k) by {
                crate::closure::key_membership(frontier.pending@,k);
                let i=choose|i:int| 0<=i<frontier.pending@.len() && frontier.pending@[i].0==k;
                if old(frontier).pending@.contains(frontier.pending@[i]) {let j=choose|j:int| 0<=j<old(frontier).pending@.len() && old(frontier).pending@[j]==frontier.pending@[i];crate::closure::key_membership(old(frontier).pending@,k);}
            }
        }
        return Ok(None);
    }
    let Some(reference) = reference else {
        return Err(Error::Missing);
    };
    let mut children = Vec::new();
    match crate::closure_node::kind(&reference) {
        Kind::Manifest => {
            if let Err(e) = source.point("before_staging_manifest_read") {
                return Err(Error::Read(e));
            }
            let manifest = match source.manifest(&reference) {
                Ok(m) => m,
                Err(e) => return Err(Error::Read(e)),
            };
            if let Err(e) = source.point("after_staging_manifest_read") {
                return Err(Error::Read(e));
            }
            let Some(dependencies) =
                crate::catalog_json::get(&manifest, "dependencies").and_then(Value::as_array)
            else {
                return Err(Error::Dependencies);
            };
            let Some(records) =
                crate::catalog_json::get(&manifest, "records").and_then(Value::as_array)
            else {
                return Err(Error::Records);
            };
            if let Err(e) = fresh_children(
                digester,
                frontier,
                &mut children,
                dependencies,
                #[cfg(straw_verus)]
                verus_exec_expr!(Ghost(source@)),
            ) {
                return Err(Error::Digest(e));
            }
            if let Err(e) = fresh_children(
                digester,
                frontier,
                &mut children,
                records,
                #[cfg(straw_verus)]
                verus_exec_expr!(Ghost(source@)),
            ) {
                return Err(Error::Digest(e));
            }
            #[cfg(straw_verus)]
            proof! {
                assert(crate::closure_node::successors(manifest)==records@+dependencies@);
                ids_add(records@,dependencies@,source@.key);
            }
        }
        Kind::Record => {}
        Kind::Invalid => return Err(Error::Reference),
    }
    #[cfg(straw_verus)]
    proof! {
        assert forall|i:int| 0<=i<frontier.pending@.len() implies source@.nodes.contains_key(frontier.pending@[i].0)
            && match frontier.pending@[i].1 {Some(v)=>(source@.key)(v)==frontier.pending@[i].0,None=>known@.contains_key(frontier.pending@[i].0)} by {
            if old(frontier).pending@.contains(frontier.pending@[i]) {let j=choose|j:int| 0<=j<old(frontier).pending@.len() && old(frontier).pending@[j]==frontier.pending@[i];}
        }
        assert forall|k:String| crate::closure::keys(frontier.pending@).contains(k) implies crate::closure::keys(old(frontier).pending@).union(source@.nodes[*key].children).contains(k) by {
            crate::closure::key_membership(frontier.pending@,k);
            let i=choose|i:int| 0<=i<frontier.pending@.len() && frontier.pending@[i].0==k;
            if old(frontier).pending@.contains(frontier.pending@[i]) {let j=choose|j:int| 0<=j<old(frontier).pending@.len() && old(frontier).pending@[j]==frontier.pending@[i];crate::closure::key_membership(old(frontier).pending@,k);}
        }
    }
    Ok(Some(children))
}

#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::environment::string_hash_contract(),crate::environment::string_content_contract(),
        crate::closure::valid(old(source)@),agrees(known@,old(source)@),binding(old(source)@,old(digester)@),
        forall|v:Value| crate::closure_node::reference_kind(v)==Kind::Record && old(source)@.nodes.contains_key((old(source)@.key)(v))
            ==> old(source)@.nodes[(old(source)@.key)(v)].children==Set::<String>::empty(),
        forall|i:int| 0<=i<references.len() ==> old(source)@.nodes.contains_key((old(source)@.key)(references@[i])),
    ensures final(source)@==old(source)@,final(digester)@==old(digester)@,
        result.is_ok() ==> result->Ok_0.nodes@==reached(old(source)@,roots(references@,old(source)@.key,Kind::Manifest))
            && result->Ok_0.raw@==roots(references@,old(source)@.key,Kind::Record)
            && result->Ok_0.additions@.dom()==result->Ok_0.nodes@.difference(known@.dom())
            && (forall|key:String| result->Ok_0.additions@.contains_key(key) ==> result->Ok_0.additions@[key]@.to_set()==old(source)@.nodes[key].children)
            && crate::consumption::compatible_graph(known@,result->Ok_0.additions@),
))]
pub fn plan<S: Source, D: Digest>(
    source: &mut S,
    digester: &mut D,
    known: &HashMap<String, Vec<String>>,
    references: &[Value],
) -> Result<Plan, Error<S::Error, D::Error>> {
    let mut frontier = crate::traversal::start();
    let mut raw = HashSet::new();
    let mut i = 0usize;
    #[cfg_attr(straw_verus,verus_spec(
        invariant i<=references.len(),source@==old(source)@,digester@==old(digester)@,
            crate::environment::string_hash_contract(),crate::environment::string_content_contract(),
            binding(source@,digester@),crate::traversal::valid(&frontier),bound(source@,known@,frontier.pending@),
            frontier.seen@==roots(references@.take(i as int),source@.key,Kind::Manifest),
            crate::closure::keys(frontier.pending@)==frontier.seen@,
            raw@==roots(references@.take(i as int),source@.key,Kind::Record),
            forall|j:int| 0<=j<references.len() ==> source@.nodes.contains_key((source@.key)(references@[j])),
        decreases references.len()-i,
    ))]
    while i < references.len() {
        let reference = &references[i];
        let kind = crate::closure_node::kind(reference);
        if let Kind::Invalid = kind {
            return Err(Error::Reference);
        }
        let key = match name(digester, reference) {
            Ok(k) => k,
            Err(e) => return Err(Error::Digest(e)),
        };
        #[cfg(straw_verus)]
        proof! {
            assert(key==(source@.key)(*reference));
            roots_push(references@.take(i as int),source@.key,*reference,Kind::Manifest);
            roots_push(references@.take(i as int),source@.key,*reference,Kind::Record);
            assert(references@.take(i as int+1)==references@.take(i as int).push(*reference));
            assert(roots(references@.take(i as int+1),source@.key,Kind::Manifest)==if kind==Kind::Manifest {frontier.seen@.insert(key)} else {frontier.seen@});
            assert(roots(references@.take(i as int+1),source@.key,Kind::Record)==if kind==Kind::Record {raw@.insert(key)} else {raw@});
        }
        if let Kind::Manifest = kind {
            let value = Some(reference.clone());
            #[cfg(straw_verus)]
            proof! {crate::closure::keys_push(frontier.pending@,key,value);}
            crate::traversal::schedule(&mut frontier, key, value);
        } else {
            raw.insert(key);
        }
        #[cfg(straw_verus)]
        proof! {
            assert(frontier.seen@==roots(references@.take(i as int+1),source@.key,Kind::Manifest));
            assert(raw@==roots(references@.take(i as int+1),source@.key,Kind::Record));
        }
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! {assert(references@.take(i as int)==references@);}
    let mut additions = HashMap::<String, Vec<String>>::new();
    #[cfg(straw_verus)]
    let initial = verus_exec_expr!(Ghost(frontier.seen@));
    #[cfg(straw_verus)]
    let mut visited = verus_exec_expr!(Ghost(Set::<String>::empty()));
    #[cfg(straw_verus)]
    proof! {
        assert forall|key:String| initial@.contains(key) implies reached(source@,initial@).contains(key) by {
            crate::closure::key_membership(frontier.pending@,key);
            let i=choose|i:int| 0<=i<frontier.pending@.len() && frontier.pending@[i].0==key;
            root_reached(source@,initial@,key);
        }
    }
    #[cfg_attr(straw_verus,verus_spec(
        invariant source@==old(source)@,digester@==old(digester)@,crate::closure::valid(source@),agrees(known@,source@),binding(source@,digester@),
            crate::environment::string_hash_contract(),crate::environment::string_content_contract(),
            forall|v:Value| crate::closure_node::reference_kind(v)==Kind::Record && source@.nodes.contains_key((source@.key)(v)) ==> source@.nodes[(source@.key)(v)].children==Set::<String>::empty(),
            initial@==roots(references@,source@.key,Kind::Manifest),raw@==roots(references@,source@.key,Kind::Record),
            crate::traversal::valid(&frontier),bound(source@,known@,frontier.pending@),
            frontier.seen@==visited@.union(crate::closure::keys(frontier.pending@)),visited@.disjoint(crate::closure::keys(frontier.pending@)),
            initial@.subset_of(frontier.seen@),frontier.seen@.subset_of(reached(source@,initial@)),
            visited@.subset_of(source@.nodes.dom()),
            forall|key:String| #[trigger] visited@.contains(key) ==> source@.nodes[key].children.subset_of(frontier.seen@),
            additions@.dom()==visited@.difference(known@.dom()),
            forall|key:String| additions@.contains_key(key) ==> additions@[key]@.to_set()==source@.nodes[key].children,
        decreases source@.nodes.dom().difference(visited@).len(),
    ))]
    loop {
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(frontier));
        #[cfg(straw_verus)]
        proof! {if frontier.pending@.len()>0 {crate::closure::pending_pop(frontier);}}
        let Some((key, reference)) = crate::traversal::next(&mut frontier) else {
            #[cfg(straw_verus)]
            proof! {
                assert(before@.pending@.len()==0);
                assert(crate::closure::keys(before@.pending@) =~= Set::<String>::empty());
                assert(crate::closure::keys(frontier.pending@) =~= Set::<String>::empty());
                assert(visited@.union(Set::<String>::empty()) =~= visited@);
                assert(visited@==frontier.seen@);
                assert forall|node:String| reached(source@,initial@).contains(node) implies frontier.seen@.contains(node) by {
                    let root=choose|root:String| initial@.contains(root) && crate::closure::reaches(source@,root,node);
                    crate::reference_validation::all_reached(source@,root,frontier.seen@);
                }
                assert(frontier.seen@ =~= reached(source@,initial@));
                assert forall|node:String| additions@.contains_key(node) implies crate::consumption::compatible_entry(known@,additions@,node) by {
                    assert forall|i:int| 0<=i<additions@[node]@.len() implies known@.contains_key(additions@[node]@[i]) || additions@.contains_key(additions@[node]@[i]) by {
                        assert(additions@[node]@.to_set().contains(additions@[node]@[i]));
                    }
                }
            }
            return Ok(Plan {
                nodes: frontier.seen,
                raw,
                additions,
            });
        };
        #[cfg(straw_verus)]
        proof! {
            assert(before@.pending@.last().0==key);crate::closure::key_membership(before@.pending@,key);
            assert(!visited@.contains(key));assert(frontier.seen@.contains(key));
            assert(frontier.seen@ =~= visited@.insert(key).union(crate::closure::keys(frontier.pending@)));
        }
        let children = expand(source, digester, known, &mut frontier, &key, reference)?;
        if let Some(children) = children {
            additions.insert(key.clone(), children);
        }
        #[cfg(straw_verus)]
        proof! {
            assert forall|child:String| source@.nodes[key].children.contains(child) implies reached(source@,initial@).contains(child) by {child_reached(source@,initial@,key,child);}
            assert(frontier.seen@ =~= visited@.insert(key).union(crate::closure::keys(frontier.pending@)));
            source@.nodes.dom().lemma_set_insert_diff_decreases(visited@,key);
            *visited.borrow_mut()=visited@.insert(key);
        }
    }
}

#[cfg(straw_verus)]
use vstd::std_specs::iter::IteratorSpec;
#[cfg_attr(straw_verus,verus_spec(result=>
    requires vstd::std_specs::hash::obeys_key_model::<K>(),
    ensures crate::collection_specs::set_remaining(result).to_set()==names@,
        crate::collection_specs::set_remaining(result).no_duplicates(),
))]
fn owned_names<K>(names: HashSet<K>) -> std::collections::hash_set::IntoIter<K> {
    #[cfg(straw_verus)]
    proof! {broadcast use vstd::std_specs::hash::axiom_random_state_builds_valid_hashers;
    assert(vstd::std_specs::hash::builds_valid_hashers::<std::collections::hash_map::RandomState>());}
    names.into_iter()
}

#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::environment::string_hash_contract(),crate::environment::string_order_contract(),
    ensures result==!known@.dom().intersect(names@).is_empty(),
        crate::json_specs::object(*final(updates))==crate::json_specs::object(*old(updates)).union_prefer_right(
        Map::new(known@.dom().intersect(names@),|k:String| Value::Null)),
))]
fn owner_updates(
    names: HashSet<String>,
    known: &HashMap<String, std::collections::BTreeSet<String>>,
    updates: &mut serde_json::Map<String, Value>,
) -> bool {
    #[cfg(straw_verus)]
    proof! {
        broadcast use vstd::std_specs::hash::axiom_random_state_builds_valid_hashers;
        assert(vstd::std_specs::hash::builds_valid_hashers::<std::collections::hash_map::RandomState>());
    }
    let mut entries = owned_names(names);
    let mut any = false;
    #[cfg(straw_verus)]
    let original = verus_exec_expr!(Ghost(crate::collection_specs::set_remaining(entries)));
    #[cfg(straw_verus)]
    let mut i = verus_exec_expr!(Ghost(0nat));
    #[cfg_attr(straw_verus,verus_spec(
        invariant i@<=original@.len(),original@.to_set()==names@,
            any==!known@.dom().intersect(original@.take(i@ as int).to_set()).is_empty(),
            crate::environment::string_hash_contract(),crate::environment::string_order_contract(),
            crate::collection_specs::set_remaining(entries)==original@.subrange(i@ as int,original@.len() as int),
            crate::json_specs::object(*updates)==crate::json_specs::object(*old(updates)).union_prefer_right(
                Map::new(known@.dom().intersect(original@.take(i@ as int).to_set()),|k:String| Value::Null)),
        decreases crate::collection_specs::set_remaining(entries).len(),
    ))]
    loop {
        let Some(owner) = entries.next() else {
            #[cfg(straw_verus)]
            proof! {assert(i@==original@.len());assert(original@.take(i@ as int) =~= original@);}
            return any;
        };
        #[cfg(straw_verus)]
        proof! {
            assert(owner==original@[i@ as int]);
            original@.take(i@ as int).lemma_push_to_set_commute(owner);
            assert(original@.take(i@ as int+1)==original@.take(i@ as int).push(owner));
        }
        if known.contains_key(&owner) {
            updates.insert(owner, Value::Null);
            any = true;
        }
        #[cfg(straw_verus)]
        proof! {let next=i@+1;*i.borrow_mut()=next;}
    }
}

#[cfg(straw_verus)]
verus! {
    pub proof fn pop_items<T>(items:Seq<T>)
        requires items.len()>0,
        ensures items.to_set()==items.drop_first().to_set().insert(items[0]),
    {
        assert forall|v:T| items.to_set().contains(v)==items.drop_first().to_set().insert(items[0]).contains(v) by {
            if items.to_set().contains(v) {
                let i=choose|i:int| 0<=i<items.len() && items[i]==v;
                if i>0 {assert(items.drop_first()[i-1]==v);}
            } else if items.drop_first().to_set().contains(v) {
                let i=choose|i:int| 0<=i<items.drop_first().len() && items.drop_first()[i]==v;
                assert(items[i+1]==v);
            }
        }
        assert(items.to_set() =~= items.drop_first().to_set().insert(items[0]));
    }
    pub open spec fn edge_object(value:Value,edges:Map<String,Vec<String>>)->bool {
        value is Object && crate::json_update::fields(value).dom()==edges.dom()
            && forall|key:String| edges.contains_key(key) ==> crate::catalog_json::strings_match(crate::json_update::fields(value)[key],edges[key]@)
    }
    pub open spec fn receipt_view(receipt:Option<&str>)->Option<Seq<char>> {match receipt {Some(id)=>Some(id@),None=>None}}
    pub open spec fn event(value:Value,names:Set<String>,edges:Map<String,Vec<String>>,receipt:Option<Seq<char>>)->bool {
        value is Object && crate::json_specs::field(value,"owners"@).is_some()
            && crate::json_specs::field(value,"owners"@).unwrap() is Object
            && crate::json_update::fields(crate::json_specs::field(value,"owners"@).unwrap())==Map::new(names,|k:String| Value::Null)
            && (if edges.dom().is_empty() {crate::json_specs::field(value,"staging_edges"@).is_none()}
                else {crate::json_specs::field(value,"staging_edges"@).is_some() && edge_object(crate::json_specs::field(value,"staging_edges"@).unwrap(),edges)})
            && (match receipt {
                None=>crate::json_specs::field(value,"consumed_handoffs"@).is_none(),
                Some(id)=>crate::json_specs::field(value,"consumed_handoffs"@).is_some()
                    && crate::json_specs::json(crate::json_specs::field(value,"consumed_handoffs"@).unwrap()) is Array
                    && crate::json_specs::json(crate::json_specs::field(value,"consumed_handoffs"@).unwrap())->Array_0.len()==1
                    && crate::json_specs::json(crate::json_specs::json(crate::json_specs::field(value,"consumed_handoffs"@).unwrap())->Array_0[0])==crate::json_specs::Json::Text(id),
            })
            && crate::json_specs::field(value,"path_encoding"@).is_some()
            && crate::json_specs::json(crate::json_specs::field(value,"path_encoding"@).unwrap())==crate::json_specs::Json::Number(Some(1))
            && forall|name:Seq<char>| name!="owners"@ && name!="staging_edges"@ && name!="consumed_handoffs"@ && name!="path_encoding"@ ==> crate::json_specs::field(value,name).is_none()
    }
}

#[cfg_attr(straw_verus,verus_spec(result=>ensures crate::catalog_json::strings_match(result,values@)))]
fn string_array(values: Vec<String>) -> Value {
    let mut entries = values.into_iter();
    let mut output = Vec::new();
    #[cfg_attr(straw_verus,verus_spec(
        invariant entries.decrease().is_some(),output@.len()<=values@.len(),
            entries.remaining()==values@.skip(output@.len() as int),
            forall|i:int| 0<=i<output@.len() ==> output@[i] is String && crate::json_specs::json(output@[i])==crate::json_specs::Json::Text(values@[i]@),
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        let Some(name) = entries.next() else {
            return Value::Array(output);
        };
        output.push(Value::String(name));
    }
}

#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::environment::string_hash_contract(),crate::environment::string_order_contract(),
    ensures edge_object(result,edges@),
))]
fn edge_values(edges: HashMap<String, Vec<String>>) -> Value {
    let mut entries = crate::catalog_decode::owned_entries(edges);
    let mut object = serde_json::Map::new();
    #[cfg(straw_verus)]
    let mut visited = verus_exec_expr!(Ghost(Set::<String>::empty()));
    #[cfg(straw_verus)]
    proof! {crate::json_specs::map_pairs_unfold(crate::json_specs::entries(object));assert(crate::json_specs::object(object)==Map::<String,Value>::empty());}
    #[cfg_attr(straw_verus,verus_spec(
        invariant crate::environment::string_hash_contract(),crate::environment::string_order_contract(),
            crate::collection_specs::remaining(entries).to_set().subset_of(edges@.kv_pairs()),
            visited@.subset_of(edges@.dom()),crate::json_specs::object(object).dom()==visited@,
            forall|key:String| edges@.contains_key(key) ==> visited@.contains(key) || crate::collection_specs::remaining(entries).to_set().contains((key,edges@[key])),
            forall|key:String| visited@.contains(key) ==> crate::catalog_json::strings_match(crate::json_specs::object(object)[key],edges@[key]@),
        decreases crate::collection_specs::remaining(entries).len(),
    ))]
    loop {
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(crate::collection_specs::remaining(entries)));
        let Some((key, children)) = entries.next() else {
            #[cfg(straw_verus)]
            proof! {assert(crate::collection_specs::remaining(entries).to_set() =~= Set::<(String,Vec<String>)>::empty());assert(visited@ =~= edges@.dom());}
            return Value::Object(object);
        };
        #[cfg(straw_verus)]
        proof! {
            assert(before@.to_set().contains((key,children)));assert(edges@.contains_key(key));assert(edges@[key]==children);pop_items(before@);
            assert(before@.to_set() =~= crate::collection_specs::remaining(entries).to_set().insert((key,children)));
        }
        object.insert(key.clone(), string_array(children));
        #[cfg(straw_verus)]
        proof! {*visited.borrow_mut()=visited@.insert(key);}
    }
}

#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::environment::string_hash_contract(),crate::environment::string_order_contract(),
    ensures result.is_none()==(known@.dom().intersect(plan.nodes@.union(plan.raw@)).is_empty() && plan.additions@.is_empty() && receipt.is_none()),
        result.is_some() ==> event(result.unwrap(),known@.dom().intersect(plan.nodes@.union(plan.raw@)),plan.additions@,receipt_view(receipt)),
))]
pub fn value(
    plan: Plan,
    known: &HashMap<String, std::collections::BTreeSet<String>>,
    receipt: Option<&str>,
) -> Option<Value> {
    let mut updates = serde_json::Map::new();
    #[cfg(straw_verus)]
    proof! {crate::json_specs::map_pairs_unfold(crate::json_specs::entries(updates));assert(crate::json_specs::object(updates)==Map::<String,Value>::empty());}
    let first = owner_updates(plan.nodes, known, &mut updates);
    let second = owner_updates(plan.raw, known, &mut updates);
    #[cfg(straw_verus)]
    proof! {assert(crate::json_specs::object(updates) =~= Map::new(known@.dom().intersect(plan.nodes@.union(plan.raw@)),|k:String| Value::Null));}
    if !first && !second && plan.additions.is_empty() && receipt.is_none() {
        return None;
    }
    let mut object = serde_json::Map::new();
    #[cfg(straw_verus)]
    proof! {assert forall|key:Seq<char>| crate::json_specs::field_in(crate::json_specs::entries(object),key).is_none() by {};}
    object.insert("owners".to_owned(), Value::Object(updates));
    if !plan.additions.is_empty() {
        object.insert("staging_edges".to_owned(), edge_values(plan.additions));
    }
    if let Some(receipt) = receipt {
        object.insert(
            "consumed_handoffs".to_owned(),
            Value::Array(vec![Value::String(receipt.to_owned())]),
        );
    }
    object.insert("path_encoding".to_owned(), Value::Number(1u64.into()));
    Some(Value::Object(object))
}
