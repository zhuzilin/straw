//! Authenticated index cache and exact record-envelope selection.
use serde_json::{Value, value::Index};

pub trait Source {
    type File;
    type Error;

    fn inspect(
        &mut self,
        reference: &Value,
    ) -> Result<(Self::File, Value), crate::extent_retry::Failure<Self::Error>>;
}

pub struct Cache {
    last: Option<(Value, Value)>,
}

impl Cache {
    pub fn new() -> Self {
        Self { last: None }
    }

    pub fn index(&self) -> Option<&Value> {
        match &self.last {
            Some((_, index)) => Some(index),
            None => None,
        }
    }

    pub fn load<S: Source>(
        &mut self,
        source: &mut S,
        reference: &Value,
    ) -> Result<Option<S::File>, crate::extent_retry::Failure<S::Error>> {
        if let Some((cached, _)) = &self.last
            && *cached == *reference
        {
            return Ok(None);
        }
        self.last = None;
        let (file, index) = source.inspect(reference)?;
        let cached = reference.clone();

        self.last = Some((cached, index));
        Ok(Some(file))
    }
}

pub enum Error {
    Records,
    Ordinal,
    Outside,
}

fn item<'a>(value: &'a Value, key: &str) -> &'a Value {
    match key.index_into(value) {
        Some(v) => v,
        None => &Value::Null,
    }
}

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

pub fn select(index: &Value, reference: &Value) -> Result<Value, Error> {
    Ok(item(member(index, reference)?, "envelope").clone())
}

pub struct Attempt<'a, S: Source> {
    cache: &'a mut Cache,
    source: &'a mut S,
    reference: &'a Value,
}

impl<'a, S: Source> Attempt<'a, S> {
    pub fn new(cache: &'a mut Cache, source: &'a mut S, reference: &'a Value) -> Self {
        Self {
            cache,
            source,
            reference,
        }
    }

    fn finish(self) {}
}

impl<S: Source> crate::extent_retry::Attempt for Attempt<'_, S> {
    type Output = Option<S::File>;
    type Error = S::Error;
    fn read(&mut self) -> Result<Self::Output, crate::extent_retry::Failure<Self::Error>> {
        self.cache.load(self.source, self.reference)
    }
}

pub fn load<S: Source, O: crate::extent_retry::Observer<S::Error>>(
    cache: &mut Cache,
    source: &mut S,
    reference: &Value,
    observer: &mut O,
    delays: &[u64],
) -> Result<Option<S::File>, crate::extent_retry::Error<S::Error>> {
    let mut attempt = Attempt::new(cache, source, reference);
    let result = crate::extent_retry::run(&mut attempt, observer, delays);

    attempt.finish();
    result
}

pub enum ReadError<E> {
    Load(crate::extent_retry::Error<E>),
    Select(Error),
}

fn cached_envelope(cache: &Cache, reference: &Value) -> Result<Value, Error> {
    let index = cache.index().unwrap();
    select(index, reference)
}

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

#[cfg(test)]
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
