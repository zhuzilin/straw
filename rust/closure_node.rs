//! Reference field selection and complete, ordered manifest successor binding.
//! Manifest authentication, namespace resolution and digest primitives are separate.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Manifest,
    Record,
    Invalid,
}

#[derive(Debug)]
pub enum Failure<E> {
    Segment,
    Path,
    Records,
    Dependencies,
    Digest(E),
}

#[derive(Debug)]
pub enum Admission<E> {
    Field(Failure<E>),
    Namespace(E),
    Reference,
    Reclaimed,
}

/// Decide permanent reclamation from the reference before payload I/O. Every
/// visited dependency uses this same admission path, including after cache reset.
pub(crate) fn live_pack<R: crate::catalog_decode::Resolver>(
    resolver: &mut R,
    reference: &Value,
    deleted: &std::collections::HashSet<String>,
) -> Result<String, Admission<R::Error>> {
    let manifest = match kind(reference) {
        Kind::Manifest => true,
        Kind::Record => false,
        Kind::Invalid => return Err(Admission::Reference),
    };
    let segment = match segment(reference, manifest) {
        Ok(segment) => segment,
        Err(e) => return Err(Admission::Field(e)),
    };
    let name = match crate::catalog_json::get(segment, "path") {
        Some(Value::String(name)) => name,
        _ => return Err(Admission::Field(Failure::Path)),
    };
    let pack = match resolver.resolve(name, crate::identities::Encoding::Legacy) {
        Ok(pack) => pack,
        Err(e) => return Err(Admission::Namespace(e)),
    };

    if deleted.contains(&pack) {
        return Err(Admission::Reclaimed);
    }
    Ok(pack)
}

pub trait Digest {
    type Error;

    fn digest(&mut self, value: &Value) -> Result<String, Self::Error>;
}

pub fn kind(reference: &Value) -> Kind {
    if crate::catalog_json::get(reference, "manifest").is_some() {
        Kind::Manifest
    } else if crate::catalog_json::get(reference, "segment").is_some() {
        Kind::Record
    } else {
        Kind::Invalid
    }
}

pub fn segment<E>(reference: &Value, manifest: bool) -> Result<&Value, Failure<E>> {
    let record = if manifest {
        match crate::catalog_json::get(reference, "manifest") {
            Some(record) => record,
            None => return Err(Failure::Segment),
        }
    } else {
        reference
    };
    match crate::catalog_json::get(record, "segment") {
        Some(segment) => Ok(segment),
        None => Err(Failure::Segment),
    }
}

#[cfg(test)]
pub fn path<E>(segment: &Value) -> Result<&str, Failure<E>> {
    let Some(value) = crate::catalog_json::get(segment, "path") else {
        return Err(Failure::Path);
    };
    match value.as_str() {
        Some(path) => Ok(path),
        None => Err(Failure::Path),
    }
}

fn array<'a>(value: &'a Value, name: &str) -> Option<&'a Vec<Value>> {
    crate::catalog_json::get(value, name)?.as_array()
}

fn append<D: Digest>(
    digester: &mut D,
    values: &[Value],
    output: &mut Vec<(String, Value)>,
) -> Result<(), D::Error> {
    let mut i = 0usize;

    while i < values.len() {
        let key = digester.digest(&values[i])?;
        output.push((key, values[i].clone()));
        i += 1;
    }
    Ok(())
}

pub fn children<D: Digest>(
    digester: &mut D,
    contents: &Value,
) -> Result<Vec<(String, Value)>, Failure<D::Error>> {
    let Some(records) = array(contents, "records") else {
        return Err(Failure::Records);
    };
    let Some(dependencies) = array(contents, "dependencies") else {
        return Err(Failure::Dependencies);
    };
    let mut output = Vec::new();
    if let Err(error) = append(digester, records, &mut output) {
        return Err(Failure::Digest(error));
    }

    if let Err(error) = append(digester, dependencies, &mut output) {
        return Err(Failure::Digest(error));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Hasher {
        calls: usize,
        fail_at: Option<usize>,
    }

    impl Digest for Hasher {
        type Error = usize;

        fn digest(&mut self, value: &Value) -> Result<String, usize> {
            self.calls += 1;
            if self.fail_at == Some(self.calls) {
                return Err(self.calls);
            }
            Ok(crate::digest(value).unwrap())
        }
    }

    #[test]
    fn manifest_presence_takes_priority_even_when_null() {
        let reference = json!({"manifest": null, "segment": {"path": "record.pack"}});
        assert_eq!(kind(&reference), Kind::Manifest);
        assert!(matches!(
            segment::<()>(&reference, true),
            Err(Failure::Segment)
        ));
        let record = segment::<()>(&reference, false).unwrap();
        assert_eq!(path::<()>(record).unwrap(), "record.pack");
        assert_eq!(kind(&Value::Null), Kind::Invalid);
    }

    #[test]
    fn both_arrays_are_validated_before_hashing() {
        let mut hasher = Hasher {
            calls: 0,
            fail_at: Some(1),
        };
        assert!(matches!(
            children(&mut hasher, &json!({"records": null})),
            Err(Failure::Records)
        ));
        assert!(matches!(
            children(&mut hasher, &json!({"records": [null], "dependencies": {}})),
            Err(Failure::Dependencies)
        ));
        assert_eq!(hasher.calls, 0);
    }

    #[test]
    fn ordered_successors_bind_duplicates_and_nested_content() {
        let record = json!({"segment": {"path": "a.pack"}, "ordinal": 1});
        let dependency = json!({"manifest": {"segment": {"path": "b.pack"}, "ordinal": 3}});
        let contents = json!({"records": [record, record], "dependencies": [dependency]});
        let mut hasher = Hasher {
            calls: 0,
            fail_at: None,
        };
        let result = children(&mut hasher, &contents).unwrap();
        assert_eq!(result.len(), 3);
        for (actual, expected) in result.iter().zip([&record, &record, &dependency]) {
            assert_eq!(&actual.1, expected);
            assert_eq!(actual.0, crate::digest(expected).unwrap());
        }
        assert_eq!(hasher.calls, 3);
    }

    #[test]
    fn digest_failure_does_not_return_a_partial_successor_list() {
        let mut hasher = Hasher {
            calls: 0,
            fail_at: Some(2),
        };
        let contents = json!({"records": ["record"], "dependencies": ["bad", "unvisited"]});
        assert!(matches!(
            children(&mut hasher, &contents),
            Err(Failure::Digest(2))
        ));
        assert_eq!(hasher.calls, 2);
    }
}
