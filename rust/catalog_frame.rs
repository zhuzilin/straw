//! Actual raw catalog replay decoder: JSON, run binding and business meaning.
use crate::catalog_decode::{self, Resolver};
use crate::catalog_state::Operation;
use std::collections::{BTreeSet, HashMap};

pub enum Error<E> {
    Json(serde_json::Error),
    Run,
    Event(catalog_decode::Failure<E>),
}

fn matches_run(event: &serde_json::Value, run: &str) -> bool {
    let Some(value) = crate::catalog_json::get(event, "run_id") else {
        return false;
    };
    match value.as_str() {
        Some(value) => value == run,
        None => false,
    }
}

pub struct Decoder<'a, R> {
    run: &'a str,
    resolver: &'a mut R,
    closures: &'a mut HashMap<String, BTreeSet<String>>,
}

impl<'a, R: Resolver> Decoder<'a, R> {
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

    pub fn finish(self) {}
}

impl<R: Resolver> crate::catalog_target::Decoder for Decoder<'_, R> {
    type Error = Error<R::Error>;

    fn reset(&mut self) {
        self.closures.clear();
        self.resolver.reset();
    }

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
            Ok(operations) => Ok(operations),
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
        fn reset(&mut self) {
            self.calls.clear();
        }
        fn resolve(&mut self, name: &str, encoding: Encoding) -> Result<String, ()> {
            self.calls.push(name.to_owned());
            match encoding {
                Encoding::Resolved => Ok(name.to_owned()),
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
