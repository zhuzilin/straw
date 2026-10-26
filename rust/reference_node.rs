//! Actual single record-set validation over normalized manifest/envelope reads.
use crate::closure_node::Digest;

use serde_json::{Value, value::Index};
use std::collections::HashSet;

pub trait Reader: crate::closure::SourceModel<Reference = Value> {
    type Error;

    fn manifest(&mut self, reference: &Value) -> Result<Value, Self::Error>;

    fn envelope(&mut self, reference: &Value) -> Result<Value, Self::Error>;
}

pub enum Error<R, D> {
    Read(R),
    Digest(D),
    Usage,
    Array(&'static str),
    Count,
    Repeated,
    Scope,
    Logical,
    Overflow,
    UsageMismatch,
}

fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    match key.index_into(value) {
        Some(v) => v,
        None => &Value::Null,
    }
}

fn count(value: &Value, key: &str) -> u64 {
    get(value, key).as_u64().unwrap_or(0)
}

fn array<'a, R, D>(value: &'a Value, key: &'static str) -> Result<&'a Vec<Value>, Error<R, D>> {
    match get(value, key).as_array() {
        Some(a) => Ok(a),
        None => Err(Error::Array(key)),
    }
}

fn copies(values: &[Value]) -> Vec<Value> {
    let mut output = Vec::new();
    let mut i = 0;

    while i < values.len() {
        output.push(values[i].clone());
        i += 1;
    }

    output
}

fn dependency_digests(dependencies: &[Value]) -> Vec<Value> {
    let mut output = Vec::new();
    let mut i = 0;

    while i < dependencies.len() {
        output.push(get(&dependencies[i], "digest").clone());
        i += 1;
    }
    output
}

fn logical_payload(envs: &[Value], dependencies: &[Value]) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("records".to_owned(), Value::Array(copies(envs)));
    object.insert(
        "dependencies".to_owned(),
        Value::Array(dependency_digests(dependencies)),
    );
    Value::Object(object)
}

fn add_values(values: &[Value], key: &str, initial: u64) -> Option<u64> {
    let mut total = initial;
    let mut i = 0;

    while i < values.len() {
        let value = count(&values[i], key);

        match crate::bounds::add(total, value) {
            Some(next) => total = next,
            None => {
                return None;
            }
        }
        i += 1;
    }

    Some(total)
}

fn usage(envs: &[Value], deps: &[Value]) -> Option<(u64, u64)> {
    let payload = add_values(envs, "length", 0)?;
    let payload = add_values(deps, "payload_bytes", payload)?;
    let tokens = add_values(envs, "tokens", 0)?;
    let tokens = add_values(deps, "tokens", tokens)?;
    Some((payload, tokens))
}

/// Return the authenticated records and dependencies of one manifest.
#[allow(clippy::type_complexity)]
pub fn validate<R: Reader, D: Digest>(
    reader: &mut R,
    digester: &mut D,
    reference: &Value,
    task: Option<&str>,
    attempt: Option<&str>,
) -> Result<(Vec<Value>, Vec<Value>), Error<R::Error, D::Error>> {
    if get(reference, "records").as_u64().is_none()
        || get(reference, "payload_bytes").as_u64().is_none()
        || get(reference, "tokens").as_u64().is_none()
    {
        return Err(Error::Usage);
    }
    let manifest = match reader.manifest(reference) {
        Ok(v) => v,
        Err(e) => return Err(Error::Read(e)),
    };
    let refs = array(&manifest, "records")?;
    if refs.len() as u64 != count(reference, "records") {
        return Err(Error::Count);
    }
    let mut known = HashSet::new();
    let mut envs = Vec::new();
    let mut i = 0;

    while i < refs.len() {
        let key = match digester.digest(&refs[i]) {
            Ok(key) => key,
            Err(e) => return Err(Error::Digest(e)),
        };
        if !known.insert(key) {
            return Err(Error::Repeated);
        }
        let env = match reader.envelope(&refs[i]) {
            Ok(value) => value,
            Err(e) => return Err(Error::Read(e)),
        };
        if let Some(wanted) = task
            && get(get(&env, "metadata"), "task_id").as_str() != Some(wanted)
        {
            return Err(Error::Scope);
        }
        if let Some(wanted) = attempt
            && get(get(&env, "metadata"), "attempt_id").as_str() != Some(wanted)
        {
            return Err(Error::Scope);
        }

        envs.push(env);
        i += 1;
    }

    let deps = array(&manifest, "dependencies")?;
    let payload = logical_payload(&envs, deps);
    let actual = match digester.digest(&payload) {
        Ok(value) => value,
        Err(e) => return Err(Error::Digest(e)),
    };
    if get(reference, "digest").as_str() != Some(actual.as_str())
        || get(&manifest, "digest").as_str() != Some(actual.as_str())
    {
        return Err(Error::Logical);
    }
    let (payload_bytes, tokens) = match usage(&envs, deps) {
        Some(v) => v,
        None => return Err(Error::Overflow),
    };
    if payload_bytes != count(reference, "payload_bytes") || tokens != count(reference, "tokens") {
        return Err(Error::UsageMismatch);
    }
    let records = copies(refs);
    let dependencies = copies(deps);

    Ok((records, dependencies))
}

pub struct Adapter<'a, R: Reader, D: Digest> {
    reader: &'a mut R,
    digester: D,
}

impl<R: Reader, D: Digest> crate::closure::SourceModel for Adapter<'_, R, D> {
    type Reference = Value;
}

impl<'a, R: Reader, D: Digest> Adapter<'a, R, D> {
    pub fn new(reader: &'a mut R, digester: D) -> Self {
        Self { reader, digester }
    }
}

impl<R: Reader, D: Digest> crate::reference_validation::Source for Adapter<'_, R, D> {
    type Error = Error<R::Error, D::Error>;
    fn node(
        &mut self,
        reference: &Value,
        task: Option<&str>,
        attempt: Option<&str>,
    ) -> Result<(Vec<Value>, Vec<Value>), Self::Error> {
        validate(self.reader, &mut self.digester, reference, task, attempt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    struct Hash;
    impl Digest for Hash {
        type Error = crate::Error;
        fn digest(&mut self, value: &Value) -> crate::Result<String> {
            crate::digest(value)
        }
    }
    struct Input {
        manifest: Value,
        envs: Vec<Value>,
        reads: Vec<Value>,
        manifests: usize,
    }
    impl crate::closure::SourceModel for Input {
        type Reference = Value;
    }
    impl Reader for Input {
        type Error = &'static str;
        fn manifest(&mut self, _: &Value) -> Result<Value, Self::Error> {
            self.manifests += 1;
            Ok(self.manifest.clone())
        }
        fn envelope(&mut self, reference: &Value) -> Result<Value, Self::Error> {
            self.reads.push(reference.clone());
            self.envs
                .get(reference.as_u64().unwrap() as usize)
                .cloned()
                .ok_or("missing envelope")
        }
    }
    fn fixture() -> (Input, Value) {
        let envs = vec![
            json!({"length":7,"tokens":3,"metadata":{"task_id":"task","attempt_id":"attempt"}}),
            json!({"length":11,"tokens":5,"metadata":{"task_id":"task","attempt_id":"attempt"}}),
        ];
        let deps = vec![json!({"digest":"child","payload_bytes":13,"tokens":17})];
        let hash = crate::digest(&json!({"records":envs,"dependencies":["child"]})).unwrap();
        (
            Input {
                manifest: json!({"records":[0,1],"dependencies":deps,"digest":hash}),
                envs,
                reads: vec![],
                manifests: 0,
            },
            json!({"records":2,"payload_bytes":31,"tokens":25,"digest":hash}),
        )
    }
    #[test]
    fn validates_exact_members_scope_digest_and_dependency_accounting() {
        let (mut source, reference) = fixture();
        let (records, deps) = validate(
            &mut source,
            &mut Hash,
            &reference,
            Some("task"),
            Some("attempt"),
        )
        .ok()
        .unwrap();
        assert_eq!(records, vec![json!(0), json!(1)]);
        assert_eq!(
            deps,
            source.manifest["dependencies"].as_array().unwrap().clone()
        );
        assert_eq!(source.reads, vec![json!(0), json!(1)]);
        assert_eq!(source.manifests, 1);
    }
    #[test]
    fn duplicate_and_scope_fail_before_unneeded_reads() {
        let (mut source, reference) = fixture();
        source.manifest["records"] = json!([0, 0]);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::Repeated)
        ));
        assert_eq!(source.reads, vec![json!(0)]);
        let (mut source, reference) = fixture();
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, Some("other"), None),
            Err(Error::Scope)
        ));
        assert_eq!(source.reads, vec![json!(0)]);
    }
    #[test]
    fn malformed_usage_and_count_fail_before_members() {
        let (mut source, mut reference) = fixture();
        reference["tokens"] = json!(-1);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::Usage)
        ));
        assert_eq!(source.manifests, 0);
        reference["tokens"] = json!(25);
        reference["records"] = json!(1);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::Count)
        ));
        assert!(source.reads.is_empty());
    }
    #[test]
    fn verifies_both_digests_before_usage_and_detects_overflow() {
        let (mut source, mut reference) = fixture();
        source.manifest["digest"] = json!("wrong");
        reference["tokens"] = json!(0);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::Logical)
        ));
        let (mut source, mut reference) = fixture();
        reference["digest"] = json!("wrong");
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::Logical)
        ));
        let (mut source, mut reference) = fixture();
        source.envs[0]["length"] = json!(u64::MAX);
        let hash = crate::digest(&json!({"records":source.envs,"dependencies":["child"]})).unwrap();
        source.manifest["digest"] = json!(hash);
        reference["digest"] = json!(hash);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::Overflow)
        ));
    }
    #[test]
    fn compares_both_usage_totals_after_valid_hash() {
        let (mut source, mut reference) = fixture();
        reference["payload_bytes"] = json!(30);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::UsageMismatch)
        ));
        let (mut source, mut reference) = fixture();
        reference["tokens"] = json!(24);
        assert!(matches!(
            validate(&mut source, &mut Hash, &reference, None, None),
            Err(Error::UsageMismatch)
        ));
    }
}
