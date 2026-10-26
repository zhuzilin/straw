//! Exact queue reference discovery and GC root selection.
//! Digest identity and authenticated transitive closure remain separate boundaries.
use crate::closure_node::Digest;
use serde_json::Value;
use serde_json::value::Index;
use std::collections::HashMap;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg(straw_verus)]
verus! {
    use crate::json_specs::{content, entries, field, json, Json, remaining};
    pub open spec fn is_reference(value: Value) -> bool {
        field(value, "manifest"@).is_some()
            && field(field(value, "manifest"@)->Some_0, "segment"@).is_some()
            && field(value, "digest"@).is_some()
    }
    pub open spec fn children(value: Value) -> Seq<Value> {
        match json(value) {
            Json::Object(items) => items.map(|i: int, pair: (String, Value)| pair.1),
            Json::Array(items) => items,
            _ => Seq::empty(),
        }
    }
    pub open spec fn selected(value: Value, root: Value, path: Seq<int>) -> bool
        decreases path.len(),
    {
        if path.len() == 0 { is_reference(value) && root == value }
        else { !is_reference(value) && 0 <= path[0] < children(value).len()
            && selected(children(value)[path[0]], root, path.drop_first()) }
    }
    pub open spec fn keys(value: Value, digest: spec_fn(Seq<u8>) -> String) -> ISet<String> {
        ISet::new(|key: String| exists|root: Value, path: Seq<int>| selected(value, root, path) && key == digest(content(root)))
    }
    pub open spec fn many_keys(items: Seq<Value>, digest: spec_fn(Seq<u8>) -> String) -> ISet<String> {
        ISet::new(|key: String| exists|i: int| 0 <= i < items.len() && keys(items[i], digest).contains(key))
    }
    pub open spec fn valid_keys(map: Map<String, Value>, digest: spec_fn(Seq<u8>) -> String) -> bool {
        forall|key: String| map.contains_key(key) ==> digest(content(map[key])) == key
    }
    pub open spec fn extension(before: Map<String, Value>, after: Map<String, Value>, allowed: ISet<String>) -> bool {
        before.dom().subset_of(after.dom()) && after.dom().to_iset().subset_of(before.dom().to_iset().union(allowed))
            && forall|key: String| before.contains_key(key) && !allowed.contains(key) ==> after[key] == before[key]
    }
    pub proof fn reference_keys(value: Value, digest: spec_fn(Seq<u8>) -> String)
        requires is_reference(value),
        ensures keys(value, digest) == iset![digest(content(value))],
    {
        assert(selected(value, value, Seq::empty()));
        assert forall|key: String| keys(value, digest).contains(key) implies key == digest(content(value)) by {
            let (root, path) = choose|root: Value, path: Seq<int>| selected(value, root, path) && key == digest(content(root));
            assert(path.len() == 0);
        }
        assert(keys(value, digest) =~= iset![digest(content(value))]);
    }
    pub proof fn descendant_keys(value: Value, digest: spec_fn(Seq<u8>) -> String)
        requires !is_reference(value),
        ensures keys(value, digest) == many_keys(children(value), digest),
    {
        assert forall|key: String| keys(value, digest).contains(key) implies many_keys(children(value), digest).contains(key) by {
            let (root, path) = choose|root: Value, path: Seq<int>| selected(value, root, path) && key == digest(content(root));
            assert(path.len() > 0);
            assert(selected(children(value)[path[0]], root, path.drop_first()));
            assert(keys(children(value)[path[0]], digest).contains(key));
        }
        assert forall|key: String| many_keys(children(value), digest).contains(key) implies keys(value, digest).contains(key) by {
            let i = choose|i: int| 0 <= i < children(value).len() && keys(children(value)[i], digest).contains(key);
            let (root, path) = choose|root: Value, path: Seq<int>| selected(children(value)[i], root, path) && key == digest(content(root));
            let full = seq![i] + path;
            assert(full.drop_first() =~= path);
            assert(selected(value, root, full));
        }
        assert(keys(value, digest) =~= many_keys(children(value), digest));
    }
    pub proof fn many_push(items: Seq<Value>, item: Value, digest: spec_fn(Seq<u8>) -> String)
        ensures many_keys(items.push(item), digest) == many_keys(items, digest).union(keys(item, digest)),
    {
        assert forall|key: String| many_keys(items.push(item), digest).contains(key) implies
            many_keys(items, digest).union(keys(item, digest)).contains(key) by {
            let i = choose|i: int| 0 <= i < items.push(item).len() && keys(items.push(item)[i], digest).contains(key);
            if i < items.len() { assert(many_keys(items, digest).contains(key)); }
        }
        assert forall|key: String| many_keys(items, digest).union(keys(item, digest)).contains(key) implies
            many_keys(items.push(item), digest).contains(key) by {
            if many_keys(items, digest).contains(key) {
                let i = choose|i: int| 0 <= i < items.len() && keys(items[i], digest).contains(key);
                assert(keys(items.push(item)[i], digest).contains(key));
            } else { assert(keys(items.push(item)[items.len() as int], digest).contains(key)); }
        }
        assert(many_keys(items.push(item), digest) =~= many_keys(items, digest).union(keys(item, digest)));
    }
    pub proof fn extension_then(before: Map<String, Value>, middle: Map<String, Value>, after: Map<String, Value>, a: ISet<String>, b: ISet<String>)
        requires extension(before, middle, a), extension(middle, after, b),
        ensures extension(before, after, a.union(b)),
    {}
    pub proof fn many_prefix(items: Seq<Value>, count: int, digest: spec_fn(Seq<u8>) -> String)
        requires 0 <= count <= items.len(),
        ensures many_keys(items.take(count), digest).subset_of(many_keys(items, digest)),
    {
        assert forall|key: String| many_keys(items.take(count), digest).contains(key) implies many_keys(items, digest).contains(key) by {
            let i = choose|i: int| 0 <= i < items.take(count).len() && keys(items.take(count)[i], digest).contains(key);
            assert(keys(items[i], digest).contains(key));
        }
    }
    pub proof fn extension_widen(before: Map<String, Value>, after: Map<String, Value>, a: ISet<String>, b: ISet<String>)
        requires extension(before, after, a), a.subset_of(b),
        ensures extension(before, after, b),
    {}
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == is_reference(*value)))]
pub fn reference(value: &Value) -> bool {
    let Some(manifest) = "manifest".index_into(value) else {
        return false;
    };
    "segment".index_into(manifest).is_some() && "digest".index_into(value).is_some()
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures final(digester)@ == old(digester)@,
        extension(old(output)@, final(output)@, keys(*value, old(digester)@)),
        valid_keys(old(output)@, old(digester)@) ==> valid_keys(final(output)@, old(digester)@),
        result.is_ok() ==> final(output)@.dom().to_iset() == old(output)@.dom().to_iset().union(keys(*value, old(digester)@)),
))]
#[cfg_attr(straw_verus, verifier::exec_allows_no_decreases_clause)]
#[allow(
    clippy::question_mark,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn references<D: Digest>(
    digester: &mut D,
    value: &Value,
    output: &mut HashMap<String, Value>,
) -> Result<(), D::Error> {
    if reference(value) {
        #[cfg(straw_verus)]
        proof! { reference_keys(*value, digester@); }
        let key = digester.digest(value)?;
        output.insert(key, value.clone());
    } else {
        #[cfg(straw_verus)]
        proof! { descendant_keys(*value, digester@); }
        if let Some(object) = value.as_object() {
            let mut it = object.iter();
            #[cfg(straw_verus)]
            let mut count = verus_exec_expr!(Ghost(0int));
            #[cfg_attr(straw_verus, verus_spec(
                invariant crate::environment::string_hash_contract(), digester@ == old(digester)@,
                    !is_reference(*value), json(*value) == Json::Object(entries(*object)),
                    0 <= count@ <= children(*value).len(),
                    remaining(it).unref() == entries(*object).skip(count@),
                    extension(old(output)@, output@, many_keys(children(*value).take(count@), digester@)),
                    valid_keys(old(output)@, digester@) ==> valid_keys(output@, digester@),
                    output@.dom().to_iset() == old(output)@.dom().to_iset().union(many_keys(children(*value).take(count@), digester@)),
                decreases remaining(it).len(),
            ))]
            loop {
                #[cfg(straw_verus)]
                let before = verus_exec_expr!(Ghost(output@));
                #[cfg(straw_verus)]
                proof! {
                    assert(remaining(it).unref().len() == remaining(it).len());
                    if remaining(it).len() > 0 {
                        assert(remaining(it).unref()[0] == (*remaining(it)[0].0, *remaining(it)[0].1));
                        assert(remaining(it).drop_first().unref() =~= remaining(it).unref().drop_first());
                    }
                }
                let Some((_, child)) = it.next() else {
                    #[cfg(straw_verus)]
                    proof! {
                        assert(count@ == children(*value).len());
                        assert(children(*value).take(count@) == children(*value));
                        descendant_keys(*value, digester@);
                    }
                    return Ok(());
                };
                #[cfg(straw_verus)]
                proof! { assert(*child == children(*value)[count@]); }
                let outcome = references(digester, child, output);
                #[cfg(straw_verus)]
                proof! {
                    extension_then(old(output)@, before@, output@, many_keys(children(*value).take(count@), digester@), keys(*child, digester@));
                    many_push(children(*value).take(count@), *child, digester@);
                    assert(children(*value).take(count@ + 1) == children(*value).take(count@).push(*child));
                    many_prefix(children(*value), count@ + 1, digester@);
                    descendant_keys(*value, digester@);
                    extension_widen(old(output)@, output@, many_keys(children(*value).take(count@ + 1), digester@), keys(*value, digester@));
                }
                if let Err(error) = outcome {
                    return Err(error);
                }
                #[cfg(straw_verus)]
                proof! { *count.borrow_mut() = count@ + 1; }
            }
        } else if let Some(items) = value.as_array() {
            let mut i = 0usize;
            #[cfg_attr(straw_verus, verus_spec(
                invariant crate::environment::string_hash_contract(), digester@ == old(digester)@,
                    !is_reference(*value), json(*value) == Json::Array(items@), i <= items.len(),
                    extension(old(output)@, output@, many_keys(items@.take(i as int), digester@)),
                    valid_keys(old(output)@, digester@) ==> valid_keys(output@, digester@),
                    output@.dom().to_iset() == old(output)@.dom().to_iset().union(many_keys(items@.take(i as int), digester@)),
                decreases items.len() - i,
            ))]
            while i < items.len() {
                #[cfg(straw_verus)]
                let before = verus_exec_expr!(Ghost(output@));
                let outcome = references(digester, &items[i], output);
                #[cfg(straw_verus)]
                proof! {
                    extension_then(old(output)@, before@, output@, many_keys(items@.take(i as int), digester@), keys(items@[i as int], digester@));
                    many_push(items@.take(i as int), items@[i as int], digester@);
                    assert(items@.take(i as int + 1) == items@.take(i as int).push(items@[i as int]));
                    many_prefix(items@, i as int + 1, digester@);
                    descendant_keys(*value, digester@);
                    extension_widen(old(output)@, output@, many_keys(items@.take(i as int + 1), digester@), keys(*value, digester@));
                }
                if let Err(error) = outcome {
                    return Err(error);
                }
                i += 1;
            }
            #[cfg(straw_verus)]
            proof! { assert(items@.take(i as int) == items@); descendant_keys(*value, digester@); }
        }
    }
    Ok(())
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy)]
pub enum Selection<'a> {
    Task,
    Commit {
        prefix: u64,
        positions: &'a std::collections::HashSet<u64>,
    },
    Batch(&'a [Value]),
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn field_or_null(value: Value, name: Seq<char>) -> Value { crate::request::field_or_null(value, name) }
    pub open spec fn finished(items: Seq<Value>, value: Value) -> bool {
        exists|i: int| 0 <= i < items.len() && crate::json_specs::equal(items[i], value)
    }
    pub open spec fn included(value: Value, selection: Selection) -> bool {
        match selection {
            Selection::Task => match crate::json_specs::text(field_or_null(value, "state"@)) {
                Some(name) => name != "completed"@ && name != "failed"@ && name != "cancelled"@,
                None => true,
            },
            Selection::Commit { prefix, positions } => crate::queue_task::count(value, "position"@) >= prefix
                && !positions@.contains(crate::queue_task::count(value, "position"@)),
            Selection::Batch(items) => !finished(items@, field_or_null(value, "batch_id"@)),
        }
    }
    pub open spec fn input(value: Value, selection: Selection) -> Value {
        match selection {
            Selection::Task => field_or_null(field_or_null(value, "spec"@), "input_ref"@),
            Selection::Commit { .. } => field_or_null(value, "result_ref"@),
            Selection::Batch(_) => value,
        }
    }
    pub open spec fn item_keys(value: Value, selection: Selection, digest: spec_fn(Seq<u8>) -> String) -> ISet<String> {
        if included(value, selection) { keys(input(value, selection), digest) } else { ISet::empty() }
    }
    pub open spec fn selection_keys(items: Seq<Value>, selection: Selection, digest: spec_fn(Seq<u8>) -> String) -> ISet<String> {
        ISet::new(|key: String| exists|i: int| 0 <= i < items.len() && item_keys(items[i], selection, digest).contains(key))
    }
    pub open spec fn object_values(value: Value) -> Seq<Value> {
        match json(value) { Json::Object(_) => children(value), _ => Seq::empty() }
    }
    pub proof fn selection_push(items: Seq<Value>, item: Value, selection: Selection, digest: spec_fn(Seq<u8>) -> String)
        ensures selection_keys(items.push(item), selection, digest) == selection_keys(items, selection, digest).union(item_keys(item, selection, digest)),
    {
        assert forall|key: String| selection_keys(items.push(item), selection, digest).contains(key) implies
            selection_keys(items, selection, digest).union(item_keys(item, selection, digest)).contains(key) by {
            let i = choose|i: int| 0 <= i < items.push(item).len() && item_keys(items.push(item)[i], selection, digest).contains(key);
            if i < items.len() { assert(selection_keys(items, selection, digest).contains(key)); }
        }
        assert forall|key: String| selection_keys(items, selection, digest).union(item_keys(item, selection, digest)).contains(key) implies
            selection_keys(items.push(item), selection, digest).contains(key) by {
            if selection_keys(items, selection, digest).contains(key) {
                let i = choose|i: int| 0 <= i < items.len() && item_keys(items[i], selection, digest).contains(key);
                assert(item_keys(items.push(item)[i], selection, digest).contains(key));
            } else { assert(item_keys(items.push(item)[items.len() as int], selection, digest).contains(key)); }
        }
        assert(selection_keys(items.push(item), selection, digest) =~= selection_keys(items, selection, digest).union(item_keys(item, selection, digest)));
    }
    pub proof fn selection_prefix(items: Seq<Value>, count: int, selection: Selection, digest: spec_fn(Seq<u8>) -> String)
        requires 0 <= count <= items.len(),
        ensures selection_keys(items.take(count), selection, digest).subset_of(selection_keys(items, selection, digest)),
    {
        assert forall|key: String| selection_keys(items.take(count), selection, digest).contains(key) implies selection_keys(items, selection, digest).contains(key) by {
            let i = choose|i: int| 0 <= i < items.take(count).len() && item_keys(items.take(count)[i], selection, digest).contains(key);
            assert(item_keys(items[i], selection, digest).contains(key));
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result => ensures *result == field_or_null(*value, name@)))]
pub(crate) fn field_value<'a>(value: &'a Value, name: &str) -> &'a Value {
    name.index_into(value).unwrap_or(&Value::Null)
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == crate::queue_task::count(*value, name@)))]
pub(crate) fn number(value: &Value, name: &str) -> u64 {
    field_value(value, name).as_u64().unwrap_or(0)
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == finished(items@, *value)))]
pub(crate) fn contains(items: &[Value], value: &Value) -> bool {
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= items.len(), forall|j: int| 0 <= j < i ==> !crate::json_specs::equal(items@[j], *value),
        decreases items.len() - i,
    ))]
    while i < items.len() {
        if Value::eq(&items[i], value) {
            return true;
        }
        i += 1;
    }
    false
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == included(*value, selection)))]
fn include(value: &Value, selection: Selection<'_>) -> bool {
    match selection {
        Selection::Task => {
            let status = field_value(value, "state").as_str();
            match status {
                Some(name) => name != "completed" && name != "failed" && name != "cancelled",
                None => true,
            }
        }
        Selection::Commit { prefix, positions } => {
            let position = number(value, "position");
            position >= prefix && !positions.contains(&position)
        }
        Selection::Batch(finished) => !contains(finished, field_value(value, "batch_id")),
    }
}

#[cfg_attr(straw_verus, verus_spec(result => ensures *result == input(*value, selection)))]
fn selected_input<'a>(value: &'a Value, selection: Selection<'_>) -> &'a Value {
    match selection {
        Selection::Task => field_value(field_value(value, "spec"), "input_ref"),
        Selection::Commit { .. } => field_value(value, "result_ref"),
        Selection::Batch(_) => value,
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures final(digester)@ == old(digester)@,
        extension(old(output)@, final(output)@, item_keys(*value, selection, old(digester)@)),
        valid_keys(old(output)@, old(digester)@) ==> valid_keys(final(output)@, old(digester)@),
        result.is_ok() ==> final(output)@.dom().to_iset() == old(output)@.dom().to_iset().union(item_keys(*value, selection, old(digester)@)),
))]
fn collect_item<D: Digest>(
    digester: &mut D,
    value: &Value,
    selection: Selection<'_>,
    output: &mut HashMap<String, Value>,
) -> Result<(), D::Error> {
    if include(value, selection) {
        references(digester, selected_input(value, selection), output)?;
    }
    Ok(())
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures final(digester)@ == old(digester)@,
        extension(old(output)@, final(output)@, selection_keys(values@, selection, old(digester)@)),
        valid_keys(old(output)@, old(digester)@) ==> valid_keys(final(output)@, old(digester)@),
        result.is_ok() ==> final(output)@.dom().to_iset() == old(output)@.dom().to_iset().union(selection_keys(values@, selection, old(digester)@)),
))]
#[allow(
    clippy::question_mark,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
fn collect_array<D: Digest>(
    digester: &mut D,
    values: &[Value],
    selection: Selection<'_>,
    output: &mut HashMap<String, Value>,
) -> Result<(), D::Error> {
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= values.len(), crate::environment::string_hash_contract(), digester@ == old(digester)@,
            extension(old(output)@, output@, selection_keys(values@.take(i as int), selection, digester@)),
            valid_keys(old(output)@, digester@) ==> valid_keys(output@, digester@),
            output@.dom().to_iset() == old(output)@.dom().to_iset().union(selection_keys(values@.take(i as int), selection, digester@)),
        decreases values.len() - i,
    ))]
    while i < values.len() {
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(output@));
        let outcome = collect_item(digester, &values[i], selection, output);
        #[cfg(straw_verus)]
        proof! {
            extension_then(old(output)@, before@, output@, selection_keys(values@.take(i as int), selection, digester@), item_keys(values@[i as int], selection, digester@));
            selection_push(values@.take(i as int), values@[i as int], selection, digester@);
            assert(values@.take(i as int + 1) == values@.take(i as int).push(values@[i as int]));
            selection_prefix(values@, i as int + 1, selection, digester@);
            extension_widen(old(output)@, output@, selection_keys(values@.take(i as int + 1), selection, digester@), selection_keys(values@, selection, digester@));
        }
        if let Err(error) = outcome {
            return Err(error);
        }
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! { assert(values@.take(i as int) == values@); }
    Ok(())
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures final(digester)@ == old(digester)@,
        extension(old(output)@, final(output)@, selection_keys(object_values(*value), selection, old(digester)@)),
        valid_keys(old(output)@, old(digester)@) ==> valid_keys(final(output)@, old(digester)@),
        result.is_ok() ==> final(output)@.dom().to_iset() == old(output)@.dom().to_iset().union(selection_keys(object_values(*value), selection, old(digester)@)),
))]
#[allow(
    clippy::question_mark,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
fn collect_object<D: Digest>(
    digester: &mut D,
    value: &Value,
    selection: Selection<'_>,
    output: &mut HashMap<String, Value>,
) -> Result<(), D::Error> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    let mut it = object.iter();
    #[cfg(straw_verus)]
    let mut count = verus_exec_expr!(Ghost(0int));
    #[cfg_attr(straw_verus, verus_spec(
        invariant crate::environment::string_hash_contract(), digester@ == old(digester)@,
            json(*value) == Json::Object(entries(*object)), 0 <= count@ <= object_values(*value).len(),
            remaining(it).unref() == entries(*object).skip(count@),
            extension(old(output)@, output@, selection_keys(object_values(*value).take(count@), selection, digester@)),
            valid_keys(old(output)@, digester@) ==> valid_keys(output@, digester@),
            output@.dom().to_iset() == old(output)@.dom().to_iset().union(selection_keys(object_values(*value).take(count@), selection, digester@)),
        decreases remaining(it).len(),
    ))]
    loop {
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(output@));
        #[cfg(straw_verus)]
        proof! {
            assert(remaining(it).unref().len() == remaining(it).len());
            if remaining(it).len() > 0 {
                assert(remaining(it).unref()[0] == (*remaining(it)[0].0, *remaining(it)[0].1));
                assert(remaining(it).drop_first().unref() =~= remaining(it).unref().drop_first());
            }
        }
        let Some((_, child)) = it.next() else {
            #[cfg(straw_verus)]
            proof! { assert(count@ == object_values(*value).len()); assert(object_values(*value).take(count@) == object_values(*value)); }
            return Ok(());
        };
        #[cfg(straw_verus)]
        proof! { assert(*child == object_values(*value)[count@]); }
        let outcome = collect_item(digester, child, selection, output);
        #[cfg(straw_verus)]
        proof! {
            extension_then(old(output)@, before@, output@, selection_keys(object_values(*value).take(count@), selection, digester@), item_keys(*child, selection, digester@));
            selection_push(object_values(*value).take(count@), *child, selection, digester@);
            assert(object_values(*value).take(count@ + 1) == object_values(*value).take(count@).push(*child));
            selection_prefix(object_values(*value), count@ + 1, selection, digester@);
            extension_widen(old(output)@, output@, selection_keys(object_values(*value).take(count@ + 1), selection, digester@), selection_keys(object_values(*value), selection, digester@));
        }
        if let Err(error) = outcome {
            return Err(error);
        }
        #[cfg(straw_verus)]
        proof! { *count.borrow_mut() = count@ + 1; }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error<E> {
    Array(&'static str),
    Digest(E),
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Roots {
    pub references: HashMap<String, Value>,
    pub prefix: u64,
    pub positions: std::collections::HashSet<u64>,
    #[cfg(straw_verus)]
    pub origin: Ghost<RootOrigin>,
}

#[cfg(straw_verus)]
verus! {
    pub type RootOrigin = (Value, spec_fn(Seq<u8>) -> String);
    impl Roots {
        pub open spec fn selected_for(&self,state:Value)->bool {
            ready(state,self.origin@.0)
                && self.prefix == crate::queue_task::count(training_consumer(state),"processed_cursor"@)
                && self.positions@.to_iset() == position_ids(array_or_empty(self.origin@.0,"processed_positions"@))
                && valid_keys(self.references@,self.origin@.1)
                && self.references@.dom().to_iset() == live_keys(state,self.origin@.0,self.origin@.1)
        }
    }
    pub open spec fn array_field(value: Value, name: Seq<char>) -> Option<Seq<Value>> {
        match json(field_or_null(value, name)) { Json::Array(items) => Some(items), _ => None }
    }
    pub open spec fn array_or_empty(value: Value, name: Seq<char>) -> Seq<Value> {
        array_field(value, name).unwrap_or(Seq::empty())
    }
    pub open spec fn position_ids(items: Seq<Value>) -> ISet<u64> {
        ISet::new(|position: u64| exists|i: int| 0 <= i < items.len() && json(items[i]) == Json::Number(Some(position)))
    }
    pub open spec fn position_one(value: Value) -> ISet<u64> {
        match json(value) { Json::Number(Some(position)) => iset![position], _ => ISet::empty() }
    }
    pub proof fn positions_push(items: Seq<Value>, item: Value)
        ensures position_ids(items.push(item)) == position_ids(items).union(position_one(item)),
    {
        assert forall|position: u64| position_ids(items.push(item)).contains(position) implies
            position_ids(items).union(position_one(item)).contains(position) by {
            let i = choose|i: int| 0 <= i < items.push(item).len() && json(items.push(item)[i]) == Json::Number(Some(position));
            if i < items.len() { assert(position_ids(items).contains(position)); }
        }
        assert forall|position: u64| position_ids(items).union(position_one(item)).contains(position) implies
            position_ids(items.push(item)).contains(position) by {
            if position_ids(items).contains(position) {
                let i = choose|i: int| 0 <= i < items.len() && json(items[i]) == Json::Number(Some(position));
                assert(json(items.push(item)[i]) == Json::Number(Some(position)));
            } else { assert(json(items.push(item)[items.len() as int]) == Json::Number(Some(position))); }
        }
        assert(position_ids(items.push(item)) =~= position_ids(items).union(position_one(item)));
    }
    pub open spec fn commit_keys(items: Seq<Value>, prefix: u64, positions: ISet<u64>, digest: spec_fn(Seq<u8>) -> String) -> ISet<String> {
        ISet::new(|key: String| exists|i: int| 0 <= i < items.len()
            && crate::queue_task::count(items[i], "position"@) >= prefix
            && !positions.contains(crate::queue_task::count(items[i], "position"@))
            && keys(field_or_null(items[i], "result_ref"@), digest).contains(key))
    }
    pub open spec fn batch_keys(items: Seq<Value>, finished_items: Seq<Value>, digest: spec_fn(Seq<u8>) -> String) -> ISet<String> {
        ISet::new(|key: String| exists|i: int| 0 <= i < items.len()
            && !finished(finished_items, field_or_null(items[i], "batch_id"@)) && keys(items[i], digest).contains(key))
    }
    pub open spec fn training_keys(state: Value, consumer: Value, progress: Value, digest: spec_fn(Seq<u8>) -> String) -> ISet<String> {
        commit_keys(array_or_empty(state, "commits"@), crate::queue_task::count(consumer, "processed_cursor"@),
            position_ids(array_or_empty(progress, "processed_positions"@)), digest)
            .union(batch_keys(object_values(field_or_null(state, "batches"@)), array_or_empty(progress, "finished_batches"@), digest))
    }
    pub open spec fn ready(state: Value, progress: Value) -> bool {
        array_field(progress, "processed_positions"@).is_some()
            && array_field(state, "commits"@).is_some() && array_field(progress, "finished_batches"@).is_some()
    }
    pub open spec fn training_consumer(state: Value) -> Value { field_or_null(field_or_null(state, "consumers"@), "training"@) }
    pub open spec fn live_keys(state: Value, progress: Value, digest: spec_fn(Seq<u8>) -> String) -> ISet<String> {
        selection_keys(object_values(field_or_null(state, "tasks"@)), Selection::Task, digest)
            .union(training_keys(state, training_consumer(state), progress, digest))
            .union(keys(field_or_null(state, "readers"@), digest))
            .union(keys(field_or_null(state, "consumers"@), digest))
            .union(keys(field_or_null(state, "producers"@), digest))
            .union(keys(field_or_null(state, "checkpoints"@), digest))
    }
    pub proof fn commit_binding(items: Seq<Value>, prefix: u64, positions: &std::collections::HashSet<u64>, digest: spec_fn(Seq<u8>) -> String)
        ensures selection_keys(items, Selection::Commit { prefix, positions }, digest)
            == commit_keys(items, prefix, positions@.to_iset(), digest),
    {
        assert(selection_keys(items, Selection::Commit { prefix, positions }, digest)
            =~= commit_keys(items, prefix, positions@.to_iset(), digest));
    }
    pub proof fn batch_binding(items: Seq<Value>, done: &[Value], digest: spec_fn(Seq<u8>) -> String)
        ensures selection_keys(items, Selection::Batch(done), digest) == batch_keys(items, done@, digest),
    {
        assert(selection_keys(items, Selection::Batch(done), digest) =~= batch_keys(items, done@, digest));
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() == array_field(*value, name@).is_some(),
        match result { Ok(items) => items@ == array_field(*value, name@)->Some_0, Err(error) => error == Error::Array(name) },
))]
pub(crate) fn array<'a, E>(
    value: &'a Value,
    name: &'static str,
) -> Result<&'a Vec<Value>, Error<E>> {
    match field_value(value, name).as_array() {
        Some(items) => Ok(items),
        None => Err(Error::Array(name)),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result@.to_iset() == position_ids(items@),
))]
fn positions(items: &[Value]) -> std::collections::HashSet<u64> {
    let mut result = std::collections::HashSet::new();
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= items.len(), result@.to_iset() == position_ids(items@.take(i as int)),
        decreases items.len() - i,
    ))]
    while i < items.len() {
        let value = items[i].as_u64();
        if let Some(position) = value {
            result.insert(position);
        }
        #[cfg(straw_verus)]
        proof! {
            positions_push(items@.take(i as int), items@[i as int]);
            assert(items@.take(i as int + 1) == items@.take(i as int).push(items@[i as int]));
            assert(position_ids(items@.take(i as int + 1)) =~= position_ids(items@.take(i as int)).union(
                match value { Some(position) => iset![position], None => ISet::empty() }));
        }
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! { assert(items@.take(i as int) == items@); }
    result
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures final(digester)@ == old(digester)@,
        valid_keys(old(output)@, old(digester)@) ==> valid_keys(final(output)@, old(digester)@),
        result.is_ok() ==> array_field(*state, "commits"@).is_some() && array_field(*progress, "finished_batches"@).is_some()
            && final(output)@.dom().to_iset() == old(output)@.dom().to_iset()
                .union(commit_keys(array_or_empty(*state, "commits"@), prefix, positions@.to_iset(), old(digester)@))
                .union(batch_keys(object_values(field_or_null(*state, "batches"@)), array_or_empty(*progress, "finished_batches"@), old(digester)@)),
))]
fn training_selected<D: Digest>(
    digester: &mut D,
    state: &Value,
    progress: &Value,
    prefix: u64,
    positions: &std::collections::HashSet<u64>,
    output: &mut HashMap<String, Value>,
) -> Result<(), Error<D::Error>> {
    let commits = array(state, "commits")?;
    if let Err(error) = collect_array(
        digester,
        commits,
        Selection::Commit { prefix, positions },
        output,
    ) {
        return Err(Error::Digest(error));
    }
    #[cfg(straw_verus)]
    proof! { commit_binding(commits@, prefix, positions, digester@); }
    #[cfg(straw_verus)]
    let middle = verus_exec_expr!(Ghost(output@));
    let finished: &[Value] = array(progress, "finished_batches")?;
    let batches = field_value(state, "batches");
    if let Err(error) = collect_object(digester, batches, Selection::Batch(finished), output) {
        return Err(Error::Digest(error));
    }
    #[cfg(straw_verus)]
    proof! { batch_binding(object_values(*batches), finished, digester@); }
    Ok(())
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures final(digester)@ == old(digester)@,
        valid_keys(old(output)@, old(digester)@) ==> valid_keys(final(output)@, old(digester)@),
        result.is_ok() ==> ready(*state, *progress)
            && final(output)@.dom().to_iset() == old(output)@.dom().to_iset().union(training_keys(*state, *consumer, *progress, old(digester)@)),
))]
pub fn training<D: Digest>(
    digester: &mut D,
    state: &Value,
    consumer: &Value,
    progress: &Value,
    output: &mut HashMap<String, Value>,
) -> Result<(), Error<D::Error>> {
    let positions = positions(array(progress, "processed_positions")?);
    training_selected(
        digester,
        state,
        progress,
        number(consumer, "processed_cursor"),
        &positions,
        output,
    )
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures final(digester)@ == old(digester)@,
        result.is_ok() ==> result->Ok_0.selected_for(*state) && ready(*state, *progress)
            && result->Ok_0.origin@ == (*progress,old(digester)@)
            && result->Ok_0.prefix == crate::queue_task::count(training_consumer(*state), "processed_cursor"@)
            && result->Ok_0.positions@.to_iset() == position_ids(array_or_empty(*progress, "processed_positions"@))
            && valid_keys(result->Ok_0.references@, old(digester)@)
            && result->Ok_0.references@.dom().to_iset() == live_keys(*state, *progress, old(digester)@),
))]
pub fn collect<D: Digest>(
    digester: &mut D,
    state: &Value,
    progress: &Value,
) -> Result<Roots, Error<D::Error>> {
    let consumer = field_value(field_value(state, "consumers"), "training");
    let prefix = number(consumer, "processed_cursor");
    let positions = positions(array(progress, "processed_positions")?);
    let mut roots = HashMap::new();
    if let Err(error) = collect_object(
        digester,
        field_value(state, "tasks"),
        Selection::Task,
        &mut roots,
    ) {
        return Err(Error::Digest(error));
    }
    training_selected(digester, state, progress, prefix, &positions, &mut roots)?;
    if let Err(error) = references(digester, field_value(state, "readers"), &mut roots) {
        return Err(Error::Digest(error));
    }
    if let Err(error) = references(digester, field_value(state, "consumers"), &mut roots) {
        return Err(Error::Digest(error));
    }
    if let Err(error) = references(digester, field_value(state, "producers"), &mut roots) {
        return Err(Error::Digest(error));
    }
    if let Err(error) = references(digester, field_value(state, "checkpoints"), &mut roots) {
        return Err(Error::Digest(error));
    }
    #[cfg(straw_verus)]
    proof! {
        assert(ready(*state,*progress));
        assert(prefix == crate::queue_task::count(training_consumer(*state),"processed_cursor"@));
        assert(positions@.to_iset() == position_ids(array_or_empty(*progress,"processed_positions"@)));
        assert(valid_keys(roots@,digester@));
        assert(roots@.dom().to_iset() == live_keys(*state,*progress,digester@));
    }
    let result = Roots {
        references: roots,
        prefix,
        positions,
        #[cfg(straw_verus)]
        origin: verus_exec_expr!(Ghost((*progress,digester@))),
    };
    #[cfg(straw_verus)]
    proof! { assert(result.selected_for(*state)); }
    Ok(result)
}

#[cfg(all(test, not(straw_verus)))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeSet;

    #[derive(Default)]
    struct Hash {
        calls: usize,
        fail_at: Option<usize>,
    }
    impl Digest for Hash {
        type Error = &'static str;
        fn digest(&mut self, value: &Value) -> Result<String, Self::Error> {
            self.calls += 1;
            if self.fail_at == Some(self.calls) {
                return Err("digest unavailable");
            }
            Ok(crate::digest(value).unwrap())
        }
    }
    fn root(name: &str) -> Value {
        json!({"manifest":{"segment":{"path":name}},"digest":name})
    }
    fn names(roots: &HashMap<String, Value>) -> BTreeSet<String> {
        for (key, value) in roots {
            assert_eq!(*key, crate::digest(value).unwrap());
        }
        roots
            .values()
            .map(|value| value["digest"].as_str().unwrap().to_owned())
            .collect()
    }
    fn expected(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }
    fn empty_state() -> Value {
        json!({"tasks":{},"commits":[],"batches":{},"readers":{},"consumers":{},"producers":{},"checkpoints":{}})
    }
    fn empty_progress() -> Value {
        json!({"processed_positions":[],"finished_batches":[]})
    }

    #[test]
    fn nested_references_preserve_existing_and_stop_below_reference() {
        let hidden = root("hidden-below-root");
        let mut outer = root("outer");
        outer["metadata"] = hidden;
        let value = json!({"a":[root("nested"),{"other":root("nested")}],"b":outer,"scalar":42});
        let previous = root("previous");
        let mut roots = HashMap::from([(crate::digest(&previous).unwrap(), previous)]);
        references(&mut Hash::default(), &value, &mut roots).unwrap();
        assert_eq!(names(&roots), expected(&["previous", "nested", "outer"]));
        assert!(reference(
            &json!({"manifest":{"segment":null},"digest":null})
        ));
        assert!(!reference(
            &json!({"manifest":{"not-segment":null},"digest":null})
        ));
    }

    #[test]
    fn traversal_propagates_error_before_later_roots() {
        let prior = root("prior");
        let mut roots = HashMap::from([(crate::digest(&prior).unwrap(), prior)]);
        let mut hash = Hash {
            calls: 0,
            fail_at: Some(2),
        };
        let result = references(
            &mut hash,
            &json!([root("first"), root("failed"), root("later")]),
            &mut roots,
        );
        assert_eq!(result, Err("digest unavailable"));
        assert_eq!(hash.calls, 2);
        assert_eq!(names(&roots), expected(&["prior", "first"]));
    }

    #[test]
    fn roots_select_every_queue_category_and_sparse_training_progress() {
        let state = json!({
            "tasks":{
                "pending":{"state":"pending","spec":{"input_ref":root("pending")}},
                "leased":{"state":"leased","spec":{"input_ref":root("continuation")}},
                "unknown":{"spec":{"input_ref":root("missing-state")}},
                "done":{"state":"completed","spec":{"input_ref":root("completed")}},
                "failed":{"state":"failed","spec":{"input_ref":root("failed")}},
                "cancelled":{"state":"cancelled","spec":{"input_ref":root("cancelled")}}
            },
            "commits":[
                {"position":1,"result_ref":root("below-prefix")},
                {"position":2,"result_ref":root("at-prefix")},
                {"position":3,"result_ref":root("processed-sparse")},
                {"position":4,"result_ref":root("later-commit")}
            ],
            "batches":{
                "open":{"batch_id":"open","plan_ref":root("batch-plan"),"ready_ref":root("batch-ready")},
                "done":{"batch_id":"done","plan_ref":root("finished-batch")}
            },
            "readers":{"old-attempt":{"input_ref":root("old-reader"),"continuation":root("continuation")}},
            "consumers":{"training":{"processed_cursor":2,"state_ref":root("consumer-state"),"progress_ref":root("consumer-progress")}},
            "producers":{"dataset":{"state_ref":root("producer-only")}},
            "checkpoints":{"step":{"checkpoint_ref":root("checkpoint"),"training_dependencies":[root("checkpoint-dependency")]}}
        });
        let progress = json!({"processed_positions":[3,3,"4",null,-1],"finished_batches":["done"]});
        let roots = collect(&mut Hash::default(), &state, &progress).unwrap();
        assert_eq!(roots.prefix, 2);
        assert_eq!(roots.positions, std::collections::HashSet::from([3]));
        assert_eq!(
            names(&roots.references),
            expected(&[
                "pending",
                "continuation",
                "missing-state",
                "at-prefix",
                "later-commit",
                "batch-plan",
                "batch-ready",
                "old-reader",
                "consumer-state",
                "consumer-progress",
                "producer-only",
                "checkpoint",
                "checkpoint-dependency"
            ])
        );
    }

    #[test]
    fn training_roots_use_argument_consumer_and_structural_batch_identity() {
        let state = json!({
            "consumers":{"training":{"processed_cursor":99}},
            "commits":[{"position":0,"result_ref":root("zero")},{"position":1,"result_ref":root("one")}],
            "batches":{"a":{"batch_id":{"part":1},"plan_ref":root("finished")},
                       "b":{"batch_id":{"part":2},"plan_ref":root("live")}}
        });
        let progress = json!({"processed_positions":[],"finished_batches":[{"part":1}]});
        let mut roots = HashMap::new();
        training(
            &mut Hash::default(),
            &state,
            &json!({"processed_cursor":1}),
            &progress,
            &mut roots,
        )
        .unwrap();
        assert_eq!(names(&roots), expected(&["one", "live"]));
    }

    #[test]
    fn malformed_arrays_keep_native_error_precedence_and_fail_closed() {
        let mut state = empty_state();
        state["tasks"]["task"] = json!({"spec":{"input_ref":root("task")}});
        state["commits"] = Value::Null;
        let mut hash = Hash::default();
        assert!(matches!(
            collect(&mut hash, &state, &Value::Null),
            Err(Error::Array("processed_positions"))
        ));
        assert_eq!(hash.calls, 0);
        assert!(matches!(
            collect(&mut hash, &state, &empty_progress()),
            Err(Error::Array("commits"))
        ));
        assert_eq!(hash.calls, 1);
        state["commits"] = json!([{"result_ref":root("commit")}]);
        let progress = json!({"processed_positions":[]});
        let mut roots = HashMap::new();
        assert!(matches!(
            training(&mut hash, &state, &Value::Null, &progress, &mut roots),
            Err(Error::Array("finished_batches"))
        ));
        assert_eq!(names(&roots), expected(&["commit"]));
        state["tasks"] = Value::Null;
        state["batches"] = json!([root("not-an-object")]);
        let result = collect(&mut hash, &state, &empty_progress()).unwrap();
        assert_eq!(names(&result.references), expected(&["commit"]));
    }
}
