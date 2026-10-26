//! Actual sparse-progress validation, source selection and immutable cache binding.
use serde_json::{Value, value::Index};
use std::{collections::HashSet, sync::Arc};

pub struct Record {
    pub metadata: Value,
    pub payload: Arc<[u8]>,
}

pub trait Reader {
    type Error;

    fn members(&mut self, reference: &Value) -> Result<Vec<Value>, Self::Error>;

    fn read(&mut self, reference: &Value) -> Result<Record, Self::Error>;
}

pub enum Error<E> {
    Read(E),
    Members,
    Codec,
    Json(serde_json::Error),
    Array(&'static str),
    Invalid,
    Duplicate,
}

pub struct Cache {
    entry: Option<(Value, Value)>,
}

impl Cache {
    pub fn new() -> Self {
        Self { entry: None }
    }
}

fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    match key.index_into(value) {
        Some(v) => v,
        None => &Value::Null,
    }
}

fn array<'a, E>(value: &'a Value, name: &'static str) -> Result<&'a Vec<Value>, Error<E>> {
    match value.as_array() {
        Some(a) => Ok(a),
        None => Err(Error::Array(name)),
    }
}

fn numeric_values(items: &[Value]) -> bool {
    let mut i = 0;

    while i < items.len() {
        if items[i].as_u64().is_none() {
            return false;
        }
        i += 1;
    }
    true
}

fn text_values(items: &[Value]) -> bool {
    let mut i = 0;

    while i < items.len() {
        if items[i].as_str().is_none() {
            return false;
        }
        i += 1;
    }
    true
}

fn unique_numbers(items: &[Value]) -> bool {
    let mut known = HashSet::new();
    let mut i = 0;

    while i < items.len() {
        let n = items[i].as_u64().unwrap();
        if !known.insert(n) {
            return false;
        }

        i += 1;
    }

    true
}

fn unique_texts(items: &[Value]) -> bool {
    let mut known: HashSet<String> = HashSet::new();
    let mut i = 0;

    while i < items.len() {
        let name = items[i].as_str().unwrap().to_owned();

        if !known.insert(name) {
            return false;
        }

        i += 1;
    }

    true
}

fn validate<E>(value: &Value) -> Result<(), Error<E>> {
    if get(value, "version").as_u64() != Some(1) {
        return Err(Error::Invalid);
    }
    let positions = array(get(value, "processed_positions"), "processed_positions")?;
    if !numeric_values(positions) {
        return Err(Error::Invalid);
    }
    let batches = array(get(value, "finished_batches"), "finished_batches")?;
    if !text_values(batches) {
        return Err(Error::Invalid);
    }
    if !unique_numbers(positions) || !unique_texts(batches) {
        return Err(Error::Duplicate);
    }
    Ok(())
}

fn empty_value() -> Value {
    let mut object = serde_json::Map::new();
    object.insert("processed_positions".to_owned(), Value::Array(Vec::new()));
    object.insert("finished_batches".to_owned(), Value::Array(Vec::new()));
    Value::Object(object)
}

pub fn load<R: Reader>(
    cache: &mut Cache,
    reader: &mut R,
    reference: &Value,
) -> Result<Value, Error<R::Error>> {
    if reference.is_null() {
        return Ok(empty_value());
    }
    if let Some((cached_ref, cached_value)) = &cache.entry
        && *cached_ref == *reference
    {
        let result = cached_value.clone();

        return Ok(result);
    }
    let members = match reader.members(reference) {
        Ok(m) => m,
        Err(e) => return Err(Error::Read(e)),
    };
    if members.len() != 1 {
        return Err(Error::Members);
    }
    let data = match reader.read(&members[0]) {
        Ok(data) => data,
        Err(e) => return Err(Error::Read(e)),
    };
    if get(&data.metadata, "codec").as_str() != Some("json.v1") {
        return Err(Error::Codec);
    }
    let parsed: Value = match serde_json::from_slice(&data.payload) {
        Ok(p) => p,
        Err(e) => return Err(Error::Json(e)),
    };
    validate(&parsed)?;

    let cached_ref = reference.clone();
    let cached_value = parsed.clone();

    cache.entry = Some((cached_ref, cached_value));
    Ok(parsed)
}

pub fn training<R: Reader>(
    cache: &mut Cache,
    reader: &mut R,
    queue: &Value,
) -> Result<Value, Error<R::Error>> {
    load(
        cache,
        reader,
        get(get(get(queue, "consumers"), "training"), "progress_ref"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[derive(Default)]
    struct Storage {
        calls: Vec<String>,
        value: Value,
        count: usize,
        codec: String,
    }
    impl Storage {
        fn valid(value: Value) -> Self {
            Self {
                value,
                count: 1,
                codec: "json.v1".into(),
                ..Self::default()
            }
        }
    }
    impl Reader for Storage {
        type Error = ();
        fn members(&mut self, reference: &Value) -> Result<Vec<Value>, ()> {
            self.calls.push(format!("members:{reference}"));
            Ok((0..self.count)
                .map(|i| json!({"root":reference,"ordinal":i}))
                .collect())
        }
        fn read(&mut self, reference: &Value) -> Result<Record, ()> {
            self.calls.push(format!("record:{reference}"));
            Ok(Record {
                metadata: json!({"codec":self.codec}),
                payload: serde_json::to_vec(&self.value).unwrap().into(),
            })
        }
    }
    fn progress(positions: Value, batches: Value) -> Value {
        json!({"version":1,"processed_positions":positions,"finished_batches":batches})
    }
    #[test]
    fn cache_tracks_the_selected_reference_and_rejects_failed_replacement() {
        let first = progress(json!([2, 9]), json!(["a"]));
        let second = progress(json!([10]), json!(["b"]));
        let mut source = Storage::valid(first.clone());
        let mut cache = Cache::new();
        let a = json!({"digest":"a"});
        let b = json!({"digest":"b"});
        assert_eq!(load(&mut cache, &mut source, &a).ok(), Some(first.clone()));
        assert_eq!(source.calls.len(), 2);
        source.value = Value::Null;
        assert_eq!(
            load(&mut cache, &mut source, &a.clone()).ok(),
            Some(first.clone())
        );
        assert_eq!(source.calls.len(), 2);
        assert!(matches!(
            load(&mut cache, &mut source, &b),
            Err(Error::Invalid)
        ));
        assert_eq!(source.calls.len(), 4);
        assert_eq!(load(&mut cache, &mut source, &a).ok(), Some(first));
        assert_eq!(source.calls.len(), 4);
        source.value = second.clone();
        assert_eq!(
            training(
                &mut cache,
                &mut source,
                &json!({"consumers":{"training":{"progress_ref":b}}})
            )
            .ok(),
            Some(second)
        );
        assert_eq!(source.calls.len(), 6);
    }
    #[test]
    fn null_progress_bypasses_cached_nonempty_state_and_storage() {
        let mut source = Storage::valid(progress(json!([7]), json!(["finished"])));
        let mut cache = Cache::new();
        assert!(load(&mut cache, &mut source, &json!({"digest":"old"})).is_ok());
        source.calls.clear();
        assert_eq!(
            load(&mut cache, &mut source, &Value::Null).ok(),
            Some(json!({"processed_positions":[],"finished_batches":[]}))
        );
        assert!(source.calls.is_empty());
    }
    #[test]
    fn progress_validation_preserves_type_duplicate_and_error_priority() {
        for (value, expected) in [
            (progress(json!([1, 1]), json!([])), "duplicate"),
            (progress(json!([]), json!(["二", "二"])), "duplicate"),
            (progress(json!([1.0]), json!([])), "invalid"),
            (progress(json!([]), json!([false])), "invalid"),
            (json!({"version":1,"processed_positions":[1,1]}), "array"),
            (json!({"version":2}), "invalid"),
        ] {
            let mut source = Storage::valid(value);
            let mut cache = Cache::new();
            let actual = match load(&mut cache, &mut source, &json!({"digest":"test"})) {
                Err(Error::Duplicate) => "duplicate",
                Err(Error::Invalid) => "invalid",
                Err(Error::Array(_)) => "array",
                _ => "unexpected",
            };
            assert_eq!(actual, expected);
            assert!(cache.entry.is_none());
        }
    }
    #[test]
    fn wrong_record_count_and_codec_are_rejected_before_parsing() {
        let mut source = Storage::valid(Value::Null);
        source.count = 2;
        let mut cache = Cache::new();
        assert!(matches!(
            load(&mut cache, &mut source, &json!({"digest":"test"})),
            Err(Error::Members)
        ));
        assert_eq!(source.calls.len(), 1);
        source.count = 1;
        source.codec = "bytes.v1".into();
        assert!(matches!(
            load(&mut cache, &mut source, &json!({"digest":"test"})),
            Err(Error::Codec)
        ));
        assert!(cache.entry.is_none());
    }
}
