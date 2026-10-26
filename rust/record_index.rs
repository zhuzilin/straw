//! Authenticated index cache and exact record-envelope selection.
use serde_json::{Value, value::Index};
#[cfg(straw_verus)]
use vstd::prelude::*;
#[cfg(straw_verus)]
verus! {
    use crate::json_specs::{field,json,Json};
    pub uninterp spec fn inspected(reference:Value,index:Value)->bool;
    pub open spec fn authenticated(reference:Value,index:Value)->bool {
        exists|source:Value| crate::queue_progress::equivalent(reference,source) && inspected(source,index)
    }
    pub open spec fn get(value:Value,key:Seq<char>)->Value {field(value,key).unwrap_or(Value::Null)}
    pub open spec fn entry(index:Value,reference:Value)->Option<Value> {
        match (json(get(index,"records"@)),json(get(reference,"ordinal"@))) {
            (Json::Array(items),Json::Number(Some(n)))=>if n<items.len() {Some(items[n as int])} else {None},
            _=>None,
        }
    }
    pub open spec fn selected(index:Value,reference:Value,envelope:Value)->bool {
        entry(index,reference).is_some() && crate::closure_node::same_reference(get(entry(index,reference)->Some_0,"envelope"@),envelope)
    }
    pub open spec fn envelope(reference:Value,value:Value)->bool {
        exists|index:Value| authenticated(get(reference,"segment"@),index) && selected(index,reference,value)
    }
    pub proof fn authenticated_copy(reference:Value,copy:Value,index:Value)
        requires authenticated(reference,index),crate::queue_progress::equivalent(reference,copy),
        ensures authenticated(copy,index),
    {
        let source=choose|source:Value| crate::queue_progress::equivalent(reference,source) && inspected(source,index);
        assert(crate::queue_progress::equivalent(copy,source));
    }
}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Source {
    type File;
    type Error;
    #[cfg_attr(straw_verus,verus_spec(result=>ensures result.is_ok() ==> inspected(*reference,result->Ok_0.1),))]
    fn inspect(
        &mut self,
        reference: &Value,
    ) -> Result<(Self::File, Value), crate::extent_retry::Failure<Self::Error>>;
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct Cache {
    last: Option<(Value, Value)>,
}
#[cfg(straw_verus)]
verus! {
    impl View for Cache {type V=Option<(Value,Value)>;closed spec fn view(&self)->Self::V {self.last}}
    impl Cache {
        pub closed spec fn valid(&self)->bool {match self.last {Some((reference,index))=>authenticated(reference,index),None=>true}}
        pub closed spec fn bound(&self,reference:Value)->bool {match self.last {Some((_,index))=>authenticated(reference,index),None=>false}}
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl Cache {
    #[cfg_attr(straw_verus,verus_spec(result=>ensures result.valid(),result@.is_none(),))]
    pub fn new() -> Self {
        Self { last: None }
    }
    #[cfg_attr(straw_verus,verus_spec(result=>ensures result.is_some()==self@.is_some(),result.is_some() ==> *result->Some_0==self@->Some_0.1,))]
    pub fn index(&self) -> Option<&Value> {
        match &self.last {
            Some((_, index)) => Some(index),
            None => None,
        }
    }
    #[cfg_attr(straw_verus,verus_spec(result=>requires old(self).valid(),ensures final(self).valid(),result.is_ok() ==> final(self).bound(*reference),result.is_err() ==> final(self)@.is_none(),
        result.is_ok() && result->Ok_0.is_none() ==> final(self)@==old(self)@,))]
    // This branch shape is shared with the cache proof and semantic controls.
    #[allow(clippy::collapsible_if)]
    pub fn load<S: Source>(
        &mut self,
        source: &mut S,
        reference: &Value,
    ) -> Result<Option<S::File>, crate::extent_retry::Failure<S::Error>> {
        if let Some((cached, _)) = &self.last {
            if *cached == *reference {
                #[cfg(straw_verus)]
                proof! {authenticated_copy(*cached,*reference,self@->Some_0.1);}
                return Ok(None);
            }
        }
        self.last = None;
        let (file, index) = source.inspect(reference)?;
        let cached = reference.clone();
        #[cfg(straw_verus)]
        proof! {assert(authenticated(*reference,index));authenticated_copy(*reference,cached,index);}
        self.last = Some((cached, index));
        Ok(Some(file))
    }
}
#[cfg_attr(straw_verus, verus_verify)]
pub enum Error {
    Records,
    Ordinal,
    Outside,
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures *result==get(*value,key@),))]
fn item<'a>(value: &'a Value, key: &str) -> &'a Value {
    match key.index_into(value) {
        Some(v) => v,
        None => &Value::Null,
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures result.is_ok()==entry(*index,*reference).is_some(),result.is_ok() ==> *result->Ok_0==entry(*index,*reference)->Some_0,))]
pub fn member<'a>(index: &'a Value, reference: &Value) -> Result<&'a Value, Error> {
    let records = match item(index, "records").as_array() {
        Some(a) => a,
        None => return Err(Error::Records),
    };
    let ordinal = match item(reference, "ordinal").as_u64() {
        Some(n) => n,
        None => return Err(Error::Ordinal),
    };
    if ordinal >= records.len() as u64 {
        return Err(Error::Outside);
    }
    let member = &records[ordinal as usize];
    Ok(member)
}

#[cfg_attr(straw_verus,verus_spec(result=>ensures result.is_ok()==entry(*index,*reference).is_some(),result.is_ok() ==> selected(*index,*reference,result->Ok_0),))]
pub fn select(index: &Value, reference: &Value) -> Result<Value, Error> {
    Ok(item(member(index, reference)?, "envelope").clone())
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Attempt<'a, S: Source> {
    cache: &'a mut Cache,
    source: &'a mut S,
    reference: &'a Value,
    #[cfg(straw_verus)]
    history: Ghost<Seq<Result<Option<S::File>, crate::extent_retry::Failure<S::Error>>>>,
}
#[cfg(straw_verus)]
verus! {
    impl<S:Source> Attempt<'_,S> {
        #[verifier::prophetic]
        pub closed spec fn returned(&self)->Cache {*final(self.cache)}
    }
    impl<S:Source> crate::extent_retry::Model for Attempt<'_,S> {
        type Output=Option<S::File>;type Error=S::Error;type Loan=(Cache,Value);
        #[verifier::prophetic]
        closed spec fn loan(&self)->(Cache,Value) {(self.returned(),*self.reference)}
        closed spec fn valid(&self)->bool {self.cache.valid() && (self.history@.len()>0 && self.history@.last() is Err ==> self.cache@.is_none())}
        closed spec fn accepts(&self,value:Self::Output)->bool {self.cache.bound(*self.reference)}
        closed spec fn attempts(&self)->Seq<Result<Self::Output,crate::extent_retry::Failure<Self::Error>>> {self.history@}
    }
}
#[cfg(not(straw_verus))]
impl<S: Source> crate::extent_retry::Model for Attempt<'_, S> {
    type Output = Option<S::File>;
    type Error = S::Error;
}
#[cfg(straw_verus)]
use crate::extent_retry::Model;
#[cfg_attr(straw_verus, verus_verify)]
impl<'a, S: Source> Attempt<'a, S> {
    #[cfg_attr(straw_verus,verus_spec(result=>requires old(cache).valid(),ensures result.valid(),result.attempts()==Seq::empty(),*final(cache)==result.returned(),result.loan().1==*reference,))]
    pub fn new(cache: &'a mut Cache, source: &'a mut S, reference: &'a Value) -> Self {
        Self {
            cache,
            source,
            reference,
            #[cfg(straw_verus)]
            history: verus_exec_expr!(Ghost(Seq::empty())),
        }
    }
    #[cfg_attr(straw_verus,verus_spec(requires self.valid(),ensures self.returned().valid(),self.history@.len()>0 && self.history@.last() is Err ==> self.returned()@.is_none(),self.accepts(None) ==> self.returned().bound(*self.reference),))]
    fn finish(self) {}
}
#[cfg_attr(straw_verus, verus_verify)]
impl<S: Source> crate::extent_retry::Attempt for Attempt<'_, S> {
    fn read(&mut self) -> Result<Self::Output, crate::extent_retry::Failure<Self::Error>> {
        let output = self.cache.load(self.source, self.reference);
        #[cfg(straw_verus)]
        proof! {*self.history.borrow_mut()=self.history@.push(output);}
        output
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>requires old(cache).valid(),ensures final(cache).valid(),result.is_ok() ==> final(cache).bound(*reference),result.is_err() ==> final(cache)@.is_none(),))]
pub fn load<S: Source, O: crate::extent_retry::Observer<S::Error>>(
    cache: &mut Cache,
    source: &mut S,
    reference: &Value,
    observer: &mut O,
    delays: &[u64],
) -> Result<Option<S::File>, crate::extent_retry::Error<S::Error>> {
    let mut attempt = Attempt::new(cache, source, reference);
    let result = crate::extent_retry::run(&mut attempt, observer, delays);
    #[cfg(straw_verus)]
    proof! {if result.is_ok() {assert(attempt.accepts(None));assert(attempt.loan().1==*reference);}}
    attempt.finish();
    result
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum ReadError<E> {
    Load(crate::extent_retry::Error<E>),
    Select(Error),
}
#[cfg_attr(straw_verus,verus_spec(result=>requires cache.valid(),cache.bound(get(*reference,"segment"@)),ensures result.is_ok() ==> envelope(*reference,result->Ok_0),))]
fn cached_envelope(cache: &Cache, reference: &Value) -> Result<Value, Error> {
    let index = cache.index().unwrap();
    select(index, reference)
}
#[cfg_attr(straw_verus,verus_spec(result=>requires old(cache).valid(),ensures final(cache).valid(),result.is_ok() ==> envelope(*reference,result->Ok_0),))]
pub fn read_envelope<S: Source, O: crate::extent_retry::Observer<S::Error>>(
    cache: &mut Cache,
    source: &mut S,
    reference: &Value,
    observer: &mut O,
    delays: &[u64],
) -> Result<Value, ReadError<S::Error>> {
    match load(cache, source, item(reference, "segment"), observer, delays) {
        Ok(_) => {}
        Err(e) => return Err(ReadError::Load(e)),
    };
    match cached_envelope(cache, reference) {
        Ok(v) => Ok(v),
        Err(e) => Err(ReadError::Select(e)),
    }
}

#[cfg(all(test, not(straw_verus)))]
mod tests {
    use super::*;
    use crate::extent_retry::{Failure, Observe};
    use serde_json::json;
    use std::collections::VecDeque;
    struct Input {
        results: VecDeque<Result<Value, Failure<&'static str>>>,
        references: Vec<Value>,
    }
    impl Source for Input {
        type File = usize;
        type Error = &'static str;
        fn inspect(&mut self, reference: &Value) -> Result<(usize, Value), Failure<&'static str>> {
            self.references.push(reference.clone());
            self.results
                .pop_front()
                .expect("unexpected index read")
                .map(|value| (self.references.len(), value))
        }
    }
    struct Silent;
    impl crate::extent_retry::Log<&'static str> for Silent {
        fn recovered(&mut self, _: usize) {}
        fn retry(&mut self, _: usize, _: usize, _: u64, _: &&'static str) {}
    }
    struct Waits<'a>(&'a mut Vec<u64>);
    impl crate::extent_retry::Wait for Waits<'_> {
        fn wait(&mut self, delay: u64) {
            self.0.push(delay);
        }
    }
    fn source(results: Vec<Result<Value, Failure<&'static str>>>) -> Input {
        Input {
            results: results.into(),
            references: vec![],
        }
    }
    #[test]
    fn cache_hit_avoids_inspect_and_failed_change_invalidates_old_index() {
        let mut input = source(vec![
            Ok(json!("first")),
            Err(Failure::Visibility("missing view")),
            Ok(json!("reread")),
        ]);
        let mut cache = Cache::new();
        assert_eq!(cache.load(&mut input, &json!("a")).ok(), Some(Some(1)));
        assert_eq!(cache.load(&mut input, &json!("a")).ok(), Some(None));
        assert!(matches!(
            cache.load(&mut input, &json!("b")),
            Err(Failure::Visibility("missing view"))
        ));
        assert!(cache.index().is_none());
        assert_eq!(cache.load(&mut input, &json!("a")).ok(), Some(Some(3)));
        assert_eq!(cache.index(), Some(&json!("reread")));
        assert_eq!(input.references, vec![json!("a"), json!("b"), json!("a")]);
    }
    #[test]
    fn typed_retry_returns_only_final_index_and_shared_budget() {
        let mut input = source(vec![
            Err(Failure::Visibility("header")),
            Err(Failure::Visibility("index")),
            Ok(json!({"records":[{"envelope":{"tokens":7}}]})),
        ]);
        let mut cache = Cache::new();
        let mut waits = vec![];
        let reference = json!({"segment":"a","ordinal":0});
        let value = read_envelope(
            &mut cache,
            &mut input,
            &reference,
            &mut Observe::new(Silent, Waits(&mut waits)),
            &[1, 2],
        )
        .ok()
        .unwrap();
        assert_eq!(value, json!({"tokens":7}));
        assert_eq!(waits, vec![1, 2]);
        assert_eq!(input.references.len(), 3);
        assert_eq!(
            read_envelope(
                &mut cache,
                &mut input,
                &reference,
                &mut Observe::new(Silent, Waits(&mut waits)),
                &[1, 2]
            )
            .ok(),
            Some(value)
        );
        assert_eq!(waits, vec![1, 2]);
        assert_eq!(input.references.len(), 3);
    }
    #[test]
    fn exhausted_and_permanent_errors_keep_final_cause_and_empty_cache() {
        let mut input = source(vec![
            Err(Failure::Visibility("one")),
            Err(Failure::Visibility("two")),
            Err(Failure::Visibility("last")),
        ]);
        let mut cache = Cache::new();
        let mut waits = vec![];
        assert!(matches!(
            load(
                &mut cache,
                &mut input,
                &json!("a"),
                &mut Observe::new(Silent, Waits(&mut waits)),
                &[1, 2]
            ),
            Err(crate::extent_retry::Error::Exhausted("last"))
        ));
        assert_eq!(waits, vec![1, 2]);
        assert!(cache.index().is_none());
        let mut input = source(vec![
            Err(Failure::Visibility("transient")),
            Err(Failure::Permanent("identity")),
        ]);
        waits.clear();
        assert!(matches!(
            load(
                &mut cache,
                &mut input,
                &json!("a"),
                &mut Observe::new(Silent, Waits(&mut waits)),
                &[1, 2]
            ),
            Err(crate::extent_retry::Error::Permanent("identity"))
        ));
        assert_eq!(waits, vec![1]);
        assert!(cache.index().is_none());
        assert_eq!(input.references.len(), 2);
    }
    #[test]
    fn envelope_selection_checks_unsigned_ordinal_before_narrowing() {
        let index = json!({"records":[{"envelope":{"value":-0.0}}]});
        for ordinal in [json!(-1), json!("0"), json!(0.0), Value::Null] {
            assert!(matches!(
                select(&index, &json!({"ordinal":ordinal})),
                Err(Error::Ordinal)
            ));
        }
        for ordinal in [1, u32::MAX as u64 + 1, u64::MAX] {
            assert!(matches!(
                select(&index, &json!({"ordinal":ordinal})),
                Err(Error::Outside)
            ));
        }
        let env = select(&index, &json!({"ordinal":0})).ok().unwrap();
        assert_eq!(serde_json::to_vec(&env).unwrap(), br#"{"value":-0.0}"#);
        assert!(matches!(
            select(&Value::Null, &json!({"ordinal":"bad"})),
            Err(Error::Records)
        ));
    }
}
