//! Actual v1/v2 record-set decoding; v2 local ordinals become scoped references.
use serde_json::{Value, value::Index};

pub enum Error {
    Json(serde_json::Error),
    Schema,
    Records,
    Dependencies,
    Ordinal,
    Precedence,
}

fn item<'a>(value: &'a Value, key: &str) -> &'a Value {
    match key.index_into(value) {
        Some(v) => v,
        None => &Value::Null,
    }
}

fn number(value: &Value, key: &str) -> u64 {
    item(value, key).as_u64().unwrap_or(0)
}

fn local_ref(parent: &Value, ordinal: &Value) -> Result<Value, Error> {
    let n = match ordinal.as_u64() {
        Some(n) => n,
        None => return Err(Error::Ordinal),
    };
    if n >= number(item(parent, "manifest"), "ordinal") {
        return Err(Error::Precedence);
    }
    let mut map = serde_json::Map::new();
    map.insert(
        "segment".to_owned(),
        item(item(parent, "manifest"), "segment").clone(),
    );
    map.insert("ordinal".to_owned(), Value::Number(n.into()));
    Ok(Value::Object(map))
}

fn convert_dependency(parent: &Value, value: &mut Value) -> Result<(), Error> {
    let Some(ordinal) = "local".index_into(value) else {
        return Ok(());
    };
    let reference = local_ref(parent, ordinal)?;
    let Value::Object(map) = value else {
        return Ok(());
    };

    map.remove("local");

    map.insert("manifest".to_owned(), reference);

    Ok(())
}

pub fn normalize(parent: &Value, metadata: &Value, mut value: Value) -> Result<Value, Error> {
    let codec = item(metadata, "codec").as_str();
    let version = number(&value, "version");
    if codec == Some("record-set.v1") && version == 1 {
        return Ok(value);
    }
    if codec != Some("record-set.v2") || version != 2 {
        return Err(Error::Schema);
    }
    let Some(records) = item(&value, "records").as_array() else {
        return Err(Error::Records);
    };
    let mut output = Vec::new();
    let mut i = 0;

    while i < records.len() {
        output.push(local_ref(parent, &records[i])?);
        i += 1;
    }
    let Value::Object(map) = &mut value else {
        return Err(Error::Records);
    };
    map.insert("records".to_owned(), Value::Array(output));
    let Some(deps) = "dependencies".index_into_mut(&mut value) else {
        return Err(Error::Dependencies);
    };
    let Value::Array(deps) = deps else {
        return Err(Error::Dependencies);
    };

    let mut i = 0;

    while i < deps.len() {
        convert_dependency(parent, &mut deps[i])?;
        i += 1;
    }
    Ok(value)
}

pub fn decode(parent: &Value, metadata: &Value, raw: &[u8]) -> Result<Value, Error> {
    let value = match serde_json::from_slice::<Value>(raw) {
        Ok(v) => v,
        Err(e) => return Err(Error::Json(e)),
    };
    normalize(parent, metadata, value)
}

pub trait Reader: crate::closure::SourceModel<Reference = Value> {
    type Error;

    fn record(&mut self, reference: &Value) -> Result<crate::queue_progress::Record, Self::Error>;
}

pub enum ReadError<E> {
    Read(E),
    Decode(Error),
}

pub fn read<R: Reader>(reader: &mut R, reference: &Value) -> Result<Value, ReadError<R::Error>> {
    let record = match reader.record(item(reference, "manifest")) {
        Ok(r) => r,
        Err(e) => return Err(ReadError::Read(e)),
    };
    let value = match decode(reference, &record.metadata, &record.payload) {
        Ok(v) => v,
        Err(e) => return Err(ReadError::Decode(e)),
    };
    Ok(value)
}

pub trait Envelopes: crate::closure::SourceModel<Reference = Value> {
    type Error;

    fn envelope(&mut self, reference: &Value) -> Result<Value, Self::Error>;
}

pub struct Adapter<'a, R: Reader + Envelopes<Error = <R as Reader>::Error>> {
    source: &'a mut R,
}

impl<R: Reader + Envelopes<Error = <R as Reader>::Error>> crate::closure::SourceModel
    for Adapter<'_, R>
{
    type Reference = Value;
}

impl<'a, R: Reader + Envelopes<Error = <R as Reader>::Error>> Adapter<'a, R> {
    pub fn new(source: &'a mut R) -> Self {
        Self { source }
    }
}

impl<R: Reader + Envelopes<Error = <R as Reader>::Error>> crate::reference_node::Reader
    for Adapter<'_, R>
{
    type Error = ReadError<<R as Reader>::Error>;
    fn manifest(&mut self, reference: &Value) -> Result<Value, Self::Error> {
        read(self.source, reference)
    }
    fn envelope(&mut self, reference: &Value) -> Result<Value, Self::Error> {
        match self.source.envelope(reference) {
            Ok(v) => Ok(v),
            Err(e) => Err(ReadError::Read(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn metadata(version: u64) -> Value {
        json!({"codec":format!("record-set.v{version}")})
    }
    #[test]
    fn v1_passes_through_and_schema_pair_is_exact() {
        let reference = json!({});
        let original = json!({"version":1,"opaque":-0.0});
        assert_eq!(
            normalize(&reference, &metadata(1), original.clone()).ok(),
            Some(original)
        );
        assert!(matches!(
            normalize(&reference, &metadata(2), json!({"version":1})),
            Err(Error::Schema)
        ));
        assert!(matches!(
            normalize(&reference, &metadata(1), json!({"version":2})),
            Err(Error::Schema)
        ));
    }
    #[test]
    fn v2_expands_local_ordinals_preserves_external_dependencies_and_fields() {
        let reference = json!({"manifest":{"segment":{"run":"run","id":"pack"},"ordinal":8}});
        let original = json!({"version":2,"records":[0,7],"dependencies":[{"local":3,"digest":"a","other":9},{"manifest":{"foreign":true},"digest":"b"}],"digest":"result","other":[1,2]});
        let value = normalize(&reference, &metadata(2), original).ok().unwrap();
        assert_eq!(
            value,
            json!({"version":2,"records":[{"segment":{"run":"run","id":"pack"},"ordinal":0},{"segment":{"run":"run","id":"pack"},"ordinal":7}],"dependencies":[{"manifest":{"segment":{"run":"run","id":"pack"},"ordinal":3},"digest":"a","other":9},{"manifest":{"foreign":true},"digest":"b"}],"digest":"result","other":[1,2]})
        );
    }
    #[test]
    fn local_references_require_unsigned_preceding_ordinals() {
        let reference = json!({"manifest":{"segment":"pack","ordinal":8}});
        for ordinal in [json!(-1), json!(1.0), json!("1"), Value::Null] {
            assert!(matches!(
                normalize(
                    &reference,
                    &metadata(2),
                    json!({"version":2,"records":[ordinal],"dependencies":[]})
                ),
                Err(Error::Ordinal)
            ));
        }
        for ordinal in [8, 9, u64::MAX] {
            assert!(matches!(
                normalize(
                    &reference,
                    &metadata(2),
                    json!({"version":2,"records":[ordinal],"dependencies":[]})
                ),
                Err(Error::Precedence)
            ));
            assert!(matches!(
                normalize(
                    &reference,
                    &metadata(2),
                    json!({"version":2,"records":[],"dependencies":[{"local":ordinal}]})
                ),
                Err(Error::Precedence)
            ));
        }
    }
    #[test]
    fn malformed_arrays_and_json_keep_error_precedence() {
        let reference = json!({"manifest":{"ordinal":8}});
        assert!(matches!(
            decode(&reference, &metadata(2), b"{"),
            Err(Error::Json(_))
        ));
        assert!(matches!(
            normalize(
                &reference,
                &metadata(2),
                json!({"version":2,"dependencies":[]})
            ),
            Err(Error::Records)
        ));
        assert!(matches!(
            normalize(&reference, &metadata(2), json!({"version":2,"records":[]})),
            Err(Error::Dependencies)
        ));
        assert!(matches!(
            normalize(
                &reference,
                &metadata(2),
                json!({"version":2,"records":[9],"dependencies":"bad"})
            ),
            Err(Error::Precedence)
        ));
    }
}
