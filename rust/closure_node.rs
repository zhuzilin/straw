//! Reference field selection and complete, ordered manifest successor binding.
//! Manifest authentication, namespace resolution and digest primitives are separate.
use serde_json::Value;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Manifest,
    Record,
    Invalid,
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Failure<E> {
    Segment,
    Path,
    Records,
    Dependencies,
    Digest(E),
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Admission<E> {
    Field(Failure<E>),
    Namespace(E),
    Reference,
    Reclaimed,
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn reference_path(reference:Value)->Option<Seq<char>> {
        match segment_value(reference,reference_kind(reference)==Kind::Manifest) {
            Some(segment)=>path_value(segment),None=>None,
        }
    }
    pub open spec fn canonical(ns:Map<String,String>,reference:Value,pack:String)->bool {
        exists|name:String| ns.contains_key(name) && reference_path(reference)==Some(name@) && ns[name]==pack
    }
}

/// Decide permanent reclamation from the reference before payload I/O. Every
/// visited dependency uses this same admission path, including after cache reset.
#[cfg_attr(straw_verus, verus_spec(result=>
    requires old(resolver).valid(),crate::environment::string_hash_contract(),
    ensures final(resolver).valid(),final(resolver)@==old(resolver)@,
        result.is_ok() ==> canonical(old(resolver)@,*reference,result->Ok_0) && !deleted@.contains(result->Ok_0),
        result==Err(Admission::Reclaimed) ==> exists|pack:String| canonical(old(resolver)@,*reference,pack) && deleted@.contains(pack),
))]
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
    #[cfg(straw_verus)]
    proof! {assert(canonical(resolver@,*reference,pack));}
    if deleted.contains(&pack) {
        return Err(Admission::Reclaimed);
    }
    Ok(pack)
}

#[cfg(straw_verus)]
verus! {
    use crate::json_specs::{field, json, content, observe, Json};
    pub trait DigestModel: View<V = spec_fn(Seq<u8>) -> String> {}
    pub open spec fn reference_kind(reference: Value) -> Kind {
        if field(reference, "manifest"@).is_some() { Kind::Manifest }
        else if field(reference, "segment"@).is_some() { Kind::Record }
        else { Kind::Invalid }
    }
    pub open spec fn segment_value(reference: Value, manifest: bool) -> Option<Value> {
        if manifest {
            match field(reference, "manifest"@) { Some(v) => field(v, "segment"@), None => None }
        } else { field(reference, "segment"@) }
    }
    pub open spec fn path_value(segment: Value) -> Option<Seq<char>> {
        match field(segment, "path"@) { Some(v) => crate::json_specs::text(v), None => None }
    }
    pub open spec fn array_value(value: Value, name: Seq<char>) -> Option<Seq<Value>> {
        match field(value, name) {
            Some(v) => match json(v) { Json::Array(items) => Some(items), _ => None },
            None => None,
        }
    }
    pub open spec fn well_formed(value: Value) -> bool {
        array_value(value, "records"@).is_some() && array_value(value, "dependencies"@).is_some()
    }
    pub open spec fn successors(value: Value) -> Seq<Value> {
        array_value(value, "records"@).unwrap() + array_value(value, "dependencies"@).unwrap()
    }
    pub open spec fn same_reference(before: Value, after: Value) -> bool {
        content(after) == content(before) && forall|depth: nat| observe(after, depth) == observe(before, depth)
    }
    pub open spec fn keyed(key: spec_fn(Seq<u8>) -> String, inputs: Seq<Value>, output: Seq<(String, Value)>) -> bool {
        output.len() == inputs.len() && forall|i: int| 0 <= i < inputs.len()
            ==> output[i].0 == key(content(inputs[i])) && same_reference(inputs[i], output[i].1)
    }
    pub open spec fn successor_keys(key: spec_fn(Seq<u8>) -> String, inputs: Seq<Value>) -> Set<String> {
        inputs.map(|i: int, value: Value| key(content(value))).to_set()
    }
    pub proof fn complete_keys(key: spec_fn(Seq<u8>) -> String, inputs: Seq<Value>, output: Seq<(String, Value)>)
        requires keyed(key, inputs, output),
        ensures crate::closure::keys(output) == successor_keys(key, inputs),
    {
        assert(output.map(|i: int, pair: (String, Value)| pair.0)
            =~= inputs.map(|i: int, value: Value| key(content(value))));
    }
    pub proof fn graph_binding(g: crate::closure::Graph<Value>, key: spec_fn(Seq<u8>) -> String,
        inputs: Seq<Value>, output: Seq<(String, Value)>)
        requires keyed(key, inputs, output),
            forall|value: Value| (g.key)(value) == key(content(value)),
            successor_keys(key, inputs).subset_of(g.nodes.dom()),
        ensures crate::closure::bound(g, output),
    {
        complete_keys(key, inputs, output);
        assert forall|i: int| 0 <= i < output.len() implies
            g.nodes.contains_key(output[i].0) && (g.key)(output[i].1) == output[i].0 by {
            crate::closure::key_membership(output, output[i].0);
            assert(crate::closure::keys(output).contains(output[i].0));
            assert(content(output[i].1) == content(inputs[i]));
        }
    }
}

#[cfg(not(straw_verus))]
pub trait DigestModel {}
#[cfg(not(straw_verus))]
impl<T> DigestModel for T {}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Digest: DigestModel {
    type Error;
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures final(self)@ == old(self)@,
            result.is_ok() ==> result->Ok_0 == (old(self)@)(crate::json_specs::content(*value)),
    ))]
    fn digest(&mut self, value: &Value) -> Result<String, Self::Error>;
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == reference_kind(*reference)))]
pub fn kind(reference: &Value) -> Kind {
    if crate::catalog_json::get(reference, "manifest").is_some() {
        Kind::Manifest
    } else if crate::catalog_json::get(reference, "segment").is_some() {
        Kind::Record
    } else {
        Kind::Invalid
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == segment_value(*reference, manifest).is_some(),
        result.is_ok() ==> *result->Ok_0 == segment_value(*reference, manifest)->Some_0,
))]
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

#[cfg(any(straw_verus, test))]
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == path_value(*segment).is_some(),
        result.is_ok() ==> result->Ok_0@ == path_value(*segment)->Some_0,
))]
pub fn path<E>(segment: &Value) -> Result<&str, Failure<E>> {
    let Some(value) = crate::catalog_json::get(segment, "path") else {
        return Err(Failure::Path);
    };
    match value.as_str() {
        Some(path) => Ok(path),
        None => Err(Failure::Path),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_some() == array_value(*value, name@).is_some(),
        result.is_some() ==> result->Some_0@ == array_value(*value, name@)->Some_0,
))]
fn array<'a>(value: &'a Value, name: &str) -> Option<&'a Vec<Value>> {
    crate::catalog_json::get(value, name)?.as_array()
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures final(digester)@ == old(digester)@,
        result.is_ok() ==> final(output)@.len() == old(output)@.len() + values@.len()
            && final(output)@.take(old(output)@.len() as int) == old(output)@
            && keyed(old(digester)@, values@, final(output)@.skip(old(output)@.len() as int)),
))]
fn append<D: Digest>(
    digester: &mut D,
    values: &[Value],
    output: &mut Vec<(String, Value)>,
) -> Result<(), D::Error> {
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= values.len(), digester@ == old(digester)@,
            output@.len() == old(output)@.len() + i,
            output@.take(old(output)@.len() as int) == old(output)@,
            keyed(digester@, values@.take(i as int), output@.skip(old(output)@.len() as int)),
        decreases values.len() - i,
    ))]
    while i < values.len() {
        let key = digester.digest(&values[i])?;
        output.push((key, values[i].clone()));
        i += 1;
    }
    Ok(())
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures final(digester)@ == old(digester)@,
        match result {
            Err(Failure::Records) => array_value(*contents, "records"@).is_none(),
            Err(Failure::Dependencies) => array_value(*contents, "records"@).is_some()
                && array_value(*contents, "dependencies"@).is_none(),
            Err(Failure::Digest(_)) => well_formed(*contents),
            Err(_) => false,
            Ok(values) => well_formed(*contents) && keyed(old(digester)@, successors(*contents), values@),
        },
))]
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
    #[cfg(straw_verus)]
    let first = verus_exec_expr!(Ghost(output@));
    if let Err(error) = append(digester, dependencies, &mut output) {
        return Err(Failure::Digest(error));
    }
    #[cfg(straw_verus)]
    proof! {
        assert forall|i: int| 0 <= i < successors(*contents).len() implies
            output@[i].0 == (digester@)(crate::json_specs::content(successors(*contents)[i]))
                && same_reference(successors(*contents)[i], output@[i].1) by {
            if i < records@.len() { assert(output@[i] == first@[i]); }
            else { assert(output@.skip(first@.len() as int)[i - first@.len()] == output@[i]); }
        }
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
