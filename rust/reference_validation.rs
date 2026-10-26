//! Actual record-set frontier, preserving the root members without rereading it.
use crate::closure_node::Digest;
use serde_json::Value;
#[cfg(straw_verus)]
use vstd::prelude::*;
#[cfg(straw_verus)]
verus! {
    pub open spec fn authenticated(key:String,task:Option<Seq<char>>,attempt:Option<Seq<char>>)->bool {crate::reference_node::authenticated(key,task,attempt)}
    pub open spec fn scope(value:Option<&str>)->Option<Seq<char>> {match value {Some(s)=>Some(s@),None=>None}}
    pub open spec fn dependency_keys(values:Seq<Value>,key:spec_fn(Value)->String)->Set<String> {values.map(|i:int,v:Value| key(v)).to_set()}
    pub proof fn dependency_member(values:Seq<Value>,key:spec_fn(Value)->String,value:Value)
        requires values.contains(value),ensures dependency_keys(values,key).contains(key(value)),
    {
        let i=choose|i:int| 0<=i<values.len() && values[i]==value;
        assert(values.map(|i:int,v:Value| key(v))[i]==key(value));
    }
    pub proof fn dependency_pop(values:Seq<Value>,key:spec_fn(Value)->String)
        requires values.len()>0,
        ensures dependency_keys(values,key)==dependency_keys(values.drop_last(),key).insert(key(values.last())),
    {
        let before=values.drop_last().map(|i:int,v:Value| key(v));
        assert(values.map(|i:int,v:Value| key(v)) =~= before.push(key(values.last())));
        before.lemma_push_to_set_commute(key(values.last()));
    }
    pub open spec fn pending_bound(g:crate::closure::Graph<Value>,pending:Seq<(String,(Value,bool))>,root:String)->bool {
        forall|i:int| 0<=i<pending.len() ==> g.nodes.contains_key(pending[i].0) && (g.key)(pending[i].1.0)==pending[i].0
            && (pending[i].1.1 == (pending[i].0==root))
    }
    pub proof fn path_closed(g:crate::closure::Graph<Value>,path:Seq<String>,seen:Set<String>)
        requires crate::closure::path(g,path),seen.contains(path[0]),
            forall|key:String| seen.contains(key) ==> g.nodes.contains_key(key) && g.nodes[key].children.subset_of(seen),
        ensures seen.contains(path.last()),decreases path.len(),
    {
        reveal(crate::closure::path);
        if path.len()>1 {assert(crate::closure::path(g,path.drop_first()));path_closed(g,path.drop_first(),seen);}
    }
    pub proof fn all_reached(g:crate::closure::Graph<Value>,root:String,seen:Set<String>)
        requires seen.contains(root),
            forall|key:String| seen.contains(key) ==> g.nodes.contains_key(key) && g.nodes[key].children.subset_of(seen),
        ensures crate::closure::nodes(g,root).subset_of(seen),
    {
        assert forall|key:String| crate::closure::nodes(g,root).contains(key) implies seen.contains(key) by {
            let path=choose|path:Seq<String>| crate::closure::path(g,path) && path[0]==root && path.last()==key;
            path_closed(g,path,seen);
        }
    }
}
#[cfg(straw_verus)]
verus! {pub trait SourceValidity {spec fn valid(&self)->bool;}}
#[cfg(not(straw_verus))]
pub trait SourceValidity {}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Source: SourceValidity + crate::closure::SourceModel<Reference = Value> {
    type Error;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires crate::record_manifest::borrowed_objects(),old(self).valid(),crate::environment::string_hash_contract(),crate::environment::string_order_contract(),crate::environment::string_content_contract(),crate::closure::valid(old(self)@),old(self)@.nodes.contains_key((old(self)@.key)(*reference)),
        ensures final(self).valid(),final(self)@==old(self)@,
            result.is_ok() ==> crate::queue_progress::manifest(*reference,result->Ok_0.0@)
                && dependency_keys(result->Ok_0.1@,old(self)@.key)==old(self)@.nodes[(old(self)@.key)(*reference)].children
                && authenticated((old(self)@.key)(*reference),scope(task),scope(attempt)),
    ))]
    fn node(
        &mut self,
        reference: &Value,
        task: Option<&str>,
        attempt: Option<&str>,
    ) -> Result<(Vec<Value>, Vec<Value>), Self::Error>;
}
#[cfg_attr(straw_verus, verus_verify)]
pub enum Error<R, D> {
    Read(R),
    Digest(D),
}

#[cfg_attr(straw_verus,verus_spec(result=>
    requires crate::record_manifest::borrowed_objects(),old(source).valid(),crate::environment::string_order_contract(),crate::environment::string_content_contract(),crate::closure::valid(old(source)@),old(source)@.nodes.contains_key((old(source)@.key)(*reference)),
        forall|v:Value| (old(source)@.key)(v)==(old(digester)@)(crate::json_specs::content(v)),
        crate::environment::string_hash_contract(),
    ensures final(source).valid(),final(source)@==old(source)@,final(digester)@==old(digester)@,
        result.is_ok() ==> crate::queue_progress::manifest(*reference,result->Ok_0@),
        result.is_ok() ==> forall|key:String| #[trigger] crate::closure::nodes(old(source)@,(old(source)@.key)(*reference)).contains(key)
            ==> authenticated(key,if key==(old(source)@.key)(*reference) {scope(task)} else {None},if key==(old(source)@.key)(*reference) {scope(attempt)} else {None}),
))]
// Keep length guards aligned with verified loop invariants and mutation anchors.
#[allow(unused_variables, clippy::len_zero)]
pub fn validate<S: Source, D: Digest>(
    source: &mut S,
    digester: &mut D,
    reference: &Value,
    task: Option<&str>,
    attempt: Option<&str>,
) -> Result<Vec<Value>, Error<S::Error, D::Error>> {
    let root_key = match digester.digest(reference) {
        Ok(key) => key,
        Err(e) => return Err(Error::Digest(e)),
    };
    #[cfg(straw_verus)]
    let root = verus_exec_expr!(Ghost(root_key));
    let mut frontier = crate::traversal::start();
    crate::traversal::schedule(&mut frontier, root_key, (reference.clone(), true));
    let mut root_records = Vec::new();
    #[cfg(straw_verus)]
    let mut visited = verus_exec_expr!(Ghost(Set::<String>::empty()));
    #[cfg(straw_verus)]
    proof! {crate::closure::root_reaches(source@,root@);crate::closure::keys_push(Seq::empty(),root@,(frontier.pending@[0].1));}
    #[cfg_attr(straw_verus,verus_spec(
        invariant crate::record_manifest::borrowed_objects(),source.valid(),crate::environment::string_order_contract(),crate::environment::string_content_contract(),source@==old(source)@,digester@==old(digester)@,crate::closure::valid(source@),
            forall|v:Value| (source@.key)(v)==(digester@)(crate::json_specs::content(v)),
            crate::environment::string_hash_contract(),root@==(source@.key)(*reference),
            crate::traversal::valid(&frontier),pending_bound(source@,frontier.pending@,root@),
            frontier.seen@==visited@.union(crate::closure::keys(frontier.pending@)),
            visited@.disjoint(crate::closure::keys(frontier.pending@)),
            frontier.seen@.contains(root@),frontier.seen@.subset_of(crate::closure::nodes(source@,root@)),
            visited@.subset_of(source@.nodes.dom()),
            forall|key:String| #[trigger] visited@.contains(key) ==> source@.nodes[key].children.subset_of(frontier.seen@)
                && authenticated(key,if key==root@ {scope(task)} else {None},if key==root@ {scope(attempt)} else {None}),
            visited@.contains(root@) ==> crate::queue_progress::manifest(*reference,root_records@),
        decreases source@.nodes.dom().difference(visited@).len(),
    ))]
    loop {
        #[cfg(straw_verus)]
        let before_frontier = verus_exec_expr!(Ghost(frontier));
        #[cfg(straw_verus)]
        let before_visited = verus_exec_expr!(Ghost(visited@));
        #[cfg(straw_verus)]
        proof! {if frontier.pending@.len()>0 {crate::closure::pending_pop(frontier);}}
        let Some((key, (value, is_root))) = crate::traversal::next(&mut frontier) else {
            #[cfg(straw_verus)]
            proof! {
                assert(before_frontier@.pending@.len()==0);
                assert(crate::closure::keys(before_frontier@.pending@) =~= Set::<String>::empty());
                assert(visited@.union(Set::<String>::empty()) =~= visited@);
                assert(crate::closure::keys(frontier.pending@)==Set::<String>::empty());
                assert(visited@==frontier.seen@);
                all_reached(source@,root@,visited@);
            }
            return Ok(root_records);
        };
        #[cfg(straw_verus)]
        proof! {
            let last=before_frontier@.pending@.len() as int-1;
            assert(before_frontier@.pending@[last].0==key);
            crate::closure::key_membership(before_frontier@.pending@,key);
            assert(crate::closure::keys(before_frontier@.pending@).contains(key));
            assert(!visited@.contains(key));assert(key==(source@.key)(value));assert(is_root==(key==root@));
            assert(frontier.seen@ =~= visited@.insert(key).union(crate::closure::keys(frontier.pending@)));
        }
        let input = if is_root { reference } else { &value };
        let (records, mut dependencies) = match source.node(
            input,
            if is_root { task } else { None },
            if is_root { attempt } else { None },
        ) {
            Ok(node) => node,
            Err(e) => return Err(Error::Read(e)),
        };
        if is_root {
            root_records = records;
        }
        #[cfg(straw_verus)]
        let all = verus_exec_expr!(Ghost(dependencies@));
        #[cfg_attr(straw_verus,verus_spec(
            invariant crate::record_manifest::borrowed_objects(),source.valid(),crate::environment::string_order_contract(),crate::environment::string_content_contract(),source@==old(source)@,digester@==old(digester)@,crate::closure::valid(source@),
                forall|v:Value| (source@.key)(v)==(digester@)(crate::json_specs::content(v)),
                crate::environment::string_hash_contract(),root@==(source@.key)(*reference),
                crate::traversal::valid(&frontier),pending_bound(source@,frontier.pending@,root@),
                frontier.seen@.contains(root@),frontier.seen@.contains(key),!visited@.contains(key),
                source@.nodes.contains_key(key),visited@.subset_of(source@.nodes.dom()),
                frontier.seen@.subset_of(crate::closure::nodes(source@,root@)),
                frontier.seen@==visited@.insert(key).union(crate::closure::keys(frontier.pending@)),
                visited@.insert(key).disjoint(crate::closure::keys(frontier.pending@)),
                dependencies@.to_set().subset_of(all@.to_set()),
                dependency_keys(all@,source@.key)==source@.nodes[key].children,
                dependency_keys(all@,source@.key).subset_of(frontier.seen@.union(dependency_keys(dependencies@,source@.key))),
                forall|done:String| #[trigger] visited@.contains(done) ==> source@.nodes[done].children.subset_of(frontier.seen@)
                    && authenticated(done,if done==root@ {scope(task)} else {None},if done==root@ {scope(attempt)} else {None}),
                authenticated(key,if key==root@ {scope(task)} else {None},if key==root@ {scope(attempt)} else {None}),
                visited@.insert(key).contains(root@) ==> crate::queue_progress::manifest(*reference,root_records@),
            decreases dependencies.len(),
        ))]
        while dependencies.len() > 0 {
            #[cfg(straw_verus)]
            let before = verus_exec_expr!(Ghost(dependencies@));
            let dependency = dependencies.pop().unwrap();
            let child = match digester.digest(&dependency) {
                Ok(key) => key,
                Err(e) => return Err(Error::Digest(e)),
            };
            #[cfg(straw_verus)]
            proof! {
                assert(before@.contains(dependency));
                assert(before@.to_set().contains(dependency));
                assert(all@.to_set().contains(dependency));
                assert(all@.contains(dependency));
                assert(child==(source@.key)(dependency));
                dependency_member(all@,source@.key,dependency);
                assert(dependency_keys(all@,source@.key).contains(child));
                crate::closure::append_edge(source@,root@,key,child);
                dependency_pop(before@,source@.key);
                assert(dependency_keys(before@,source@.key)==dependency_keys(dependencies@,source@.key).insert(child));
                crate::closure::keys_push(frontier.pending@,child,(dependency,false));
            }
            crate::traversal::schedule(&mut frontier, child, (dependency, false));
        }
        #[cfg(straw_verus)]
        proof! {
            assert(dependency_keys(dependencies@,source@.key) =~= Set::<String>::empty());
            assert(source@.nodes[key].children.subset_of(frontier.seen@));
            source@.nodes.dom().lemma_set_insert_diff_decreases(visited@,key);
            *visited.borrow_mut()=visited@.insert(key);
        }
    }
}

#[cfg(all(test, not(straw_verus)))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    struct Names;
    impl Digest for Names {
        type Error = &'static str;
        fn digest(&mut self, value: &Value) -> Result<String, Self::Error> {
            value.as_str().map(str::to_owned).ok_or("bad reference")
        }
    }
    struct Nodes {
        nodes: HashMap<String, (Vec<Value>, Vec<Value>)>,
        visits: Vec<(String, Option<String>, Option<String>)>,
    }
    impl crate::closure::SourceModel for Nodes {
        type Reference = Value;
    }
    impl SourceValidity for Nodes {}
    impl Source for Nodes {
        type Error = &'static str;
        fn node(
            &mut self,
            reference: &Value,
            task: Option<&str>,
            attempt: Option<&str>,
        ) -> Result<(Vec<Value>, Vec<Value>), Self::Error> {
            let key = reference.as_str().unwrap().to_owned();
            self.visits.push((
                key.clone(),
                task.map(str::to_owned),
                attempt.map(str::to_owned),
            ));
            self.nodes.get(&key).cloned().ok_or("unreadable node")
        }
    }
    #[test]
    fn cyclic_shared_dependencies_preserve_root_members_and_single_visits() {
        let mut source = Nodes {
            nodes: HashMap::from([
                (
                    "root".into(),
                    (
                        vec![json!("root-record")],
                        vec![json!("left"), json!("right")],
                    ),
                ),
                (
                    "left".into(),
                    (
                        vec![json!("left-record")],
                        vec![json!("root"), json!("shared")],
                    ),
                ),
                (
                    "right".into(),
                    (vec![json!("right-record")], vec![json!("shared")]),
                ),
                ("shared".into(), (vec![], vec![])),
            ]),
            visits: vec![],
        };
        let result = validate(
            &mut source,
            &mut Names,
            &json!("root"),
            Some("task"),
            Some("attempt"),
        )
        .ok()
        .unwrap();
        assert_eq!(result, vec![json!("root-record")]);
        assert_eq!(
            source.visits,
            vec![
                ("root".into(), Some("task".into()), Some("attempt".into())),
                ("left".into(), None, None),
                ("shared".into(), None, None),
                ("right".into(), None, None),
            ]
        );
    }
    #[test]
    fn failed_dependency_never_returns_partial_root_members() {
        let mut source = Nodes {
            nodes: HashMap::from([
                (
                    "root".into(),
                    (
                        vec![json!("root-record")],
                        vec![json!("missing"), json!("later")],
                    ),
                ),
                ("later".into(), (vec![], vec![])),
            ]),
            visits: vec![],
        };
        assert!(matches!(
            validate(&mut source, &mut Names, &json!("root"), None, None),
            Err(Error::Read("unreadable node"))
        ));
        assert_eq!(
            source.visits,
            vec![("root".into(), None, None), ("missing".into(), None, None)]
        );
    }
    #[test]
    fn leaf_root_is_read_once_and_returns_its_original_members() {
        let mut source = Nodes {
            nodes: HashMap::from([("root".into(), (vec![json!(1), json!(2)], vec![]))]),
            visits: vec![],
        };
        assert_eq!(
            validate(&mut source, &mut Names, &json!("root"), None, None).ok(),
            Some(vec![json!(1), json!(2)])
        );
        assert_eq!(source.visits.len(), 1);
    }
}
