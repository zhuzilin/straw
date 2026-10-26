//! Actual raw catalog replay decoder: JSON, run binding and business meaning.
use crate::catalog_decode::{self, Resolver};
use crate::catalog_state::Operation;
use std::collections::{BTreeSet, HashMap};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub enum Error<E> {
    Json(serde_json::Error),
    Run,
    Event(catalog_decode::Failure<E>),
}

#[cfg(straw_verus)]
verus! {
    pub struct Context {
        pub run: Seq<char>,
        pub namespace: Map<String, String>,
    }
    pub open spec fn same_run(value: serde_json::Value, run: Seq<char>) -> bool {
        match crate::json_specs::field(value, "run_id"@) {
            Some(v) => crate::json_specs::json(v) == crate::json_specs::Json::Text(run),
            None => false,
        }
    }
    pub open spec fn meaning(context: Context, raw: Seq<u8>,
        known: Map<String, Seq<String>>, operations: Seq<Operation>) -> bool {
        exists|value: serde_json::Value, wire: crate::catalog_json::Wire,
            keys: Map<String, Vec<String>>|
            crate::json_specs::parses(raw, value) && same_run(value, context.run)
                && crate::catalog_json::schema(value) && crate::catalog_json::wire_matches(value, wire)
                && crate::catalog_state::edge_view(keys) == known
                && catalog_decode::admitted(wire, keys)
                && catalog_decode::resolved_wire(context.namespace, wire)
                && catalog_decode::meaning(context.namespace, wire, operations)
    }
    pub trait DecoderModel: View<V = Context> {
        spec fn valid(&self) -> bool;
        type Loan;
        #[verifier::prophetic]
        spec fn loan(&self) -> Self::Loan;
    }
}
#[cfg(not(straw_verus))]
pub trait DecoderModel {}
#[cfg(not(straw_verus))]
impl<T> DecoderModel for T {}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == same_run(*event, run@)))]
fn matches_run(event: &serde_json::Value, run: &str) -> bool {
    let Some(value) = crate::catalog_json::get(event, "run_id") else {
        return false;
    };
    match value.as_str() {
        Some(value) => value == run,
        None => false,
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Decoder<'a, R> {
    run: &'a str,
    resolver: &'a mut R,
    closures: &'a mut HashMap<String, BTreeSet<String>>,
}

#[cfg(straw_verus)]
verus! {
    impl<R: Resolver> View for Decoder<'_, R> {
        type V = Context;
        closed spec fn view(&self) -> Context {
            Context { run: self.run@, namespace: self.resolver@ }
        }
    }
    impl<R: Resolver> DecoderModel for Decoder<'_, R> {
        closed spec fn valid(&self) -> bool { self.resolver.valid() }
        type Loan = (Map<String,String>,bool);
        #[verifier::prophetic]
        closed spec fn loan(&self) -> Self::Loan { (self.returned_namespace(),self.returned_valid()) }
    }
    impl<R: Resolver> Decoder<'_, R> {
        #[verifier::prophetic]
        pub closed spec fn returned_namespace(&self) -> Map<String,String> { final(self.resolver)@ }
        #[verifier::prophetic]
        pub closed spec fn returned_valid(&self) -> bool { final(self.resolver).valid() }
        pub closed spec fn closure_cache_empty(&self) -> bool { self.closures@ == Map::<String, BTreeSet<String>>::empty() }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<'a, R: Resolver> Decoder<'a, R> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(resolver).valid(),
        ensures result.valid(), result@ == (Context { run: run@, namespace: old(resolver)@ }),
            final(resolver)@ == result.returned_namespace(),
            final(resolver).valid() == result.returned_valid(),
            result.loan() == (result.returned_namespace(), result.returned_valid()),
    ))]
    pub fn new(
        run: &'a str,
        resolver: &'a mut R,
        closures: &'a mut HashMap<String, BTreeSet<String>>,
    ) -> Self {
        Self {
            run,
            resolver,
            closures,
        }
    }
    #[cfg_attr(straw_verus, verus_spec(
        requires self.valid(),
        ensures self.returned_valid(), self.returned_namespace() == self@.namespace,
            self.loan().0 == self@.namespace, self.loan().1,
    ))]
    pub fn finish(self) {}
}

#[cfg_attr(straw_verus, verus_verify)]
impl<R: Resolver> crate::catalog_target::Decoder for Decoder<'_, R> {
    type Error = Error<R::Error>;

    #[cfg_attr(straw_verus, verus_spec(ensures final(self).closure_cache_empty(), final(self).returned_valid() == old(self).returned_valid(), final(self).returned_namespace() == old(self).returned_namespace()))]
    fn reset(&mut self) {
        self.closures.clear();
        self.resolver.reset();
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures final(self).returned_valid() == old(self).returned_valid(), final(self).returned_namespace() == old(self).returned_namespace(),
            result == Err(Error::Run) ==> exists|event: serde_json::Value|
            crate::json_specs::parses(raw@, event) && !same_run(event, old(self)@.run),
    ))]
    fn decode(
        &mut self,
        raw: &[u8],
        known: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<Operation>, Self::Error> {
        let event = match serde_json::from_slice::<serde_json::Value>(raw) {
            Ok(event) => event,
            Err(error) => return Err(Error::Json(error)),
        };
        if !matches_run(&event, self.run) {
            return Err(Error::Run);
        }
        let result = catalog_decode::decode(self.resolver, &event, known);
        match result {
            Ok(operations) => {
                #[cfg(straw_verus)]
                proof! {
                    let wire = choose|wire: crate::catalog_json::Wire|
                        crate::catalog_json::wire_matches(event, wire)
                            && catalog_decode::construction_result(old(self)@.namespace, wire, known@, result);
                    assert(meaning(old(self)@, raw@, crate::catalog_state::edge_view(known@), operations@));
                }
                Ok(operations)
            }
            Err(error) => Err(Error::Event(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_target::Decoder as _;
    use crate::identities::Encoding;
    use serde_json::json;

    #[derive(Default)]
    struct Namespace {
        calls: Vec<String>,
    }
    impl Resolver for Namespace {
        type Error = ();
        type Keys = crate::gc::NativeKeySet;
        fn reset(&mut self) {
            self.calls.clear();
        }
        fn resolve(&mut self, name: &String, encoding: Encoding) -> Result<String, ()> {
            self.calls.push(name.clone());
            match encoding {
                Encoding::Resolved => Ok(name.clone()),
                Encoding::Legacy if name == "alias" => Ok("data/pack".into()),
                _ => Err(()),
            }
        }
    }

    #[test]
    fn wrong_run_never_reaches_namespace_or_business_decode() {
        let mut closures = HashMap::new();
        let mut namespace = Namespace::default();
        let mut decoder = Decoder::new("current", &mut namespace, &mut closures);
        for run in [json!("foreign"), json!(null), json!(12), json!(["current"])] {
            let raw =
                serde_json::to_vec(&json!({"run_id":run,"owners":{"reader":["alias"]}})).unwrap();
            assert!(matches!(
                decoder.decode(&raw, &HashMap::new()),
                Err(Error::Run)
            ));
            assert!(decoder.resolver.calls.is_empty());
        }
        assert!(matches!(
            decoder.decode(b"{", &HashMap::new()),
            Err(Error::Json(_))
        ));
        assert!(matches!(
            decoder.decode(b"{}", &HashMap::new()),
            Err(Error::Run)
        ));
    }

    #[test]
    fn decoded_raw_history_preserves_paths_order_and_resets_caches() {
        let mut closures =
            HashMap::from([("cached-root".into(), BTreeSet::from(["old-pack".into()]))]);
        let mut namespace = Namespace::default();
        let mut decoder = Decoder::new("current", &mut namespace, &mut closures);
        let mut indexes = crate::catalog_state::Indexes::new();
        let events = [
            json!({"run_id":"current","owners":{"reader":["alias"]},"packs":{"alias":true}}),
            json!({"run_id":"current","path_encoding":1,"owners":{"reader":null},"deleted":["data/pack"]}),
            json!({"run_id":"current","path_encoding":1,"reclaimed":["data/pack"]}),
        ];
        for event in events {
            let raw = serde_json::to_vec(&event).unwrap();
            let operations = match decoder.decode(&raw, &indexes.staging_edges) {
                Ok(operations) => operations,
                Err(_) => panic!("valid frame rejected"),
            };
            indexes.apply(&operations).unwrap();
        }
        assert!(indexes.owners.is_empty());
        assert!(indexes.packs.is_empty());
        assert!(indexes.pending_deletions.is_empty());
        assert!(indexes.deleted.contains("data/pack"));
        assert!(!decoder.resolver.calls.is_empty());
        decoder.reset();
        assert!(decoder.closures.is_empty());
        assert!(decoder.resolver.calls.is_empty());
    }
}
