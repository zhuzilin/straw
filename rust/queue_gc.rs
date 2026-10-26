//! Actual queue GC retirement barrier, owner naming and locked catalog dispatch.
//! The retention token, exact owner set and catalog append are checked code.
//! Filesystem durability, lock refresh and closure identity remain separate contracts.
#[cfg(any(test, straw_verus))]
use crate::queue_roots::Roots;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
#[cfg(straw_verus)]
use vstd::{prelude::*, std_specs::iter::IteratorSpec};

pub use crate::queue_retirement::Retired;

#[cfg(straw_verus)]
verus! {
    pub open spec fn owner_names(prefix: Seq<char>, keys: ISet<String>) -> ISet<String> {
        ISet::new(|owner: String| exists|key: String| keys.contains(key) && owner@ == prefix + key@)
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Prepared<'a> {
    prefix: &'a str,
    live: HashSet<String>,
    #[cfg(straw_verus)]
    snapshot: Ghost<Roots>,
}

#[cfg(straw_verus)]
verus! { impl View for Prepared<'_> { type V = (Seq<char>, Set<String>, Roots); closed spec fn view(&self) -> Self::V { (self.prefix@, self.live@, self.snapshot@) } } }

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(), crate::environment::string_content_contract(),
    ensures result@.to_iset() == owner_names(prefix@, roots@.dom().to_iset()),
))]
fn owners(prefix: &str, roots: &HashMap<String, Value>) -> HashSet<String> {
    let mut live = HashSet::new();
    let mut entries = roots.iter();
    #[cfg_attr(straw_verus, verus_spec(
        invariant crate::environment::string_hash_contract(), crate::environment::string_content_contract(),
            entries.decrease().is_some(), entries.remaining().unref().to_set().subset_of(roots@.kv_pairs()),
            forall|key: String| roots@.contains_key(key) ==> entries.remaining().unref().to_set().contains((key, roots@[key]))
                || exists|owner: String| live@.contains(owner) && owner@ == prefix@ + key@,
            forall|owner: String| live@.contains(owner) ==> exists|key: String| roots@.contains_key(key) && owner@ == prefix@ + key@,
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        #[cfg(straw_verus)]
        proof! { if entries.remaining().len() > 0 {
            crate::catalog_state::pop_pairs(entries.remaining());
            assert(entries.remaining().unref().to_set().contains((*entries.remaining()[0].0,*entries.remaining()[0].1)));
        } }
        let Some((key, _)) = entries.next() else {
            #[cfg(straw_verus)]
            proof! {
                assert(entries.remaining().unref().to_set() =~= Set::<(String, Value)>::empty());
                assert forall|owner: String| owner_names(prefix@, roots@.dom().to_iset()).contains(owner) implies live@.contains(owner) by {
                    let key = choose|key: String| roots@.contains_key(key) && owner@ == prefix@ + key@;
                    let existing = choose|existing: String| live@.contains(existing) && existing@ == prefix@ + key@;
                    assert(existing == owner);
                }
                assert(live@.to_iset() =~= owner_names(prefix@, roots@.dom().to_iset()));
            }
            return live;
        };
        #[cfg(straw_verus)]
        proof! { assert(roots@.contains_key(*key)); }
        let mut owner = prefix.to_owned();
        owner.push_str(key);
        #[cfg(straw_verus)]
        proof! { assert(owner@ == prefix@ + key@); }
        live.insert(owner);
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(), crate::environment::string_content_contract(),
    ensures result@.0 == prefix@, result@.2 == retired@,
        result@.1.to_iset() == owner_names(prefix@, retired@.references@.dom().to_iset()),
))]
pub fn prepare<'a>(prefix: &'a str, retired: Retired) -> Prepared<'a> {
    let roots = retired.into_roots();
    let live = owners(prefix, &roots.references);
    Prepared {
        prefix,
        live,
        #[cfg(straw_verus)]
        snapshot: verus_exec_expr!(Ghost(roots)),
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<'a> Prepared<'a> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures result.0@ == self@.0, result.1@ == self@.1,
    ))]
    pub(crate) fn parts(self) -> (&'a str, HashSet<String>) {
        (self.prefix, self.live)
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == !byte_prefix(name@, "publication:"@),
))]
pub(crate) fn not_publication(name: &str) -> bool {
    !name.as_bytes().starts_with("publication:".as_bytes())
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (forall|i:int| 0 <= i < names.len() ==> !byte_prefix(names@[i]@,"publication:"@)),
))]
pub(crate) fn not_publication_owners(names: &[String]) -> bool {
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= names.len(), forall|j:int| 0 <= j < i ==> !byte_prefix(names@[j]@,"publication:"@),
        decreases names.len()-i,
    ))]
    while i < names.len() {
        if !not_publication(&names[i]) {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (!byte_prefix(name@, "publication:"@) && !byte_prefix(name@, "queue:"@)),
))]
pub(crate) fn public_owner(name: &str) -> bool {
    not_publication(name) && !name.as_bytes().starts_with("queue:".as_bytes())
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (forall|i:int| 0 <= i < names.len() ==> !byte_prefix(names@[i]@,"publication:"@) && !byte_prefix(names@[i]@,"queue:"@)),
))]
#[cfg(straw_verus)]
pub(crate) fn public_owners(names: &[String]) -> bool {
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= names.len(), forall|j:int| 0 <= j < i ==> !byte_prefix(names@[j]@,"publication:"@) && !byte_prefix(names@[j]@,"queue:"@),
        decreases names.len()-i,
    ))]
    while i < names.len() {
        if !public_owner(&names[i]) {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn byte_prefix(owner: Seq<char>, prefix: Seq<char>) -> bool {
        let full = vstd::utf8::encode_utf8(owner);
        let head = vstd::utf8::encode_utf8(prefix);
        head.len() <= full.len() && forall|i: int| 0 <= i < head.len() ==> full[i] == head[i]
    }
    pub open spec fn dead(owners: Map<String, std::collections::BTreeSet<String>>, prefix: Seq<char>, live: Set<String>) -> Set<String> {
        owners.dom().filter(|owner: String| byte_prefix(owner@, prefix) && !live.contains(owner))
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(),
    ensures result@.to_set() == dead(owners@, prefix@, live@),
))]
pub fn prunable(
    owners: &HashMap<String, std::collections::BTreeSet<String>>,
    prefix: &str,
    live: &HashSet<String>,
) -> Vec<String> {
    let mut result = Vec::new();
    let mut entries = owners.iter();
    #[cfg_attr(straw_verus, verus_spec(
        invariant crate::environment::string_hash_contract(), entries.decrease().is_some(),
            entries.remaining().unref().to_set().subset_of(owners@.kv_pairs()),
            result@.to_set().subset_of(dead(owners@, prefix@, live@)),
            forall|key: String| dead(owners@, prefix@, live@).contains(key) ==>
                result@.to_set().contains(key) || entries.remaining().unref().to_set().contains((key, owners@[key])),
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        #[cfg(straw_verus)]
        proof! {
            if entries.remaining().len() > 0 {
                crate::catalog_state::pop_pairs(entries.remaining());
                assert(entries.remaining().unref().to_set().contains((*entries.remaining()[0].0,*entries.remaining()[0].1)));
            }
        }
        let Some((owner, _)) = entries.next() else {
            #[cfg(straw_verus)]
            proof! { assert(entries.remaining().unref().to_set() =~= Set::<(String, std::collections::BTreeSet<String>)>::empty()); }
            return result;
        };
        let discard =
            owner.as_str().as_bytes().starts_with(prefix.as_bytes()) && !live.contains(owner);
        #[cfg(straw_verus)]
        proof! { assert(owners@.contains_key(*owner)); assert(discard == dead(owners@, prefix@, live@).contains(*owner)); }
        if discard {
            #[cfg(straw_verus)]
            proof! { result@.lemma_push_to_set_commute(*owner); }
            result.push(owner.clone());
        }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnershipError {
    Missing,
    Invalid,
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn retained(owners: Map<String, std::collections::BTreeSet<String>>, deleted: Set<String>, live: Set<String>) -> bool {
        forall|owner: String| live.contains(owner) ==> owners.contains_key(owner)
            && owners[owner]@.len() > 0 && owners[owner]@.disjoint(deleted)
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
    ensures result.is_ok() == retained(owners@, deleted@, live@),
        result == Err(OwnershipError::Missing) ==> exists|owner: String| live@.contains(owner) && !owners@.contains_key(owner),
        result == Err(OwnershipError::Invalid) ==> exists|owner: String| live@.contains(owner) && owners@.contains_key(owner)
            && (owners@[owner]@.len() == 0 || !owners@[owner]@.disjoint(deleted@)),
))]
pub fn require(
    owners: &HashMap<String, std::collections::BTreeSet<String>>,
    deleted: &HashSet<String>,
    live: &HashSet<String>,
) -> Result<(), OwnershipError> {
    let mut entries = live.iter();
    #[cfg_attr(straw_verus, verus_spec(
        invariant crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
            entries.decrease().is_some(), entries.remaining().unref().to_set().subset_of(live@),
            forall|owner: String| live@.contains(owner) ==> entries.remaining().unref().to_set().contains(owner)
                || (owners@.contains_key(owner) && owners@[owner]@.len() > 0 && owners@[owner]@.disjoint(deleted@)),
        decreases entries.decrease().unwrap(),
    ))]
    loop {
        #[cfg(straw_verus)]
        proof! { if entries.remaining().len() > 0 {
            crate::catalog_state::pop_set(entries.remaining());
            assert(entries.remaining().unref().to_set().contains(*entries.remaining()[0]));
        } }
        let Some(owner) = entries.next() else {
            #[cfg(straw_verus)]
            proof! { assert(entries.remaining().unref().to_set() =~= Set::<String>::empty()); }
            return Ok(());
        };
        #[cfg(straw_verus)]
        proof! { assert(live@.contains(*owner)); }
        let Some(paths) = owners.get(owner) else {
            return Err(OwnershipError::Missing);
        };
        if paths.is_empty() || crate::closure::reclaimed(paths, deleted) {
            return Err(OwnershipError::Invalid);
        }
    }
}

#[cfg(straw_verus)]
verus! {
    use crate::json_specs::{entries, field, field_in, map_pairs};
    pub open spec fn unique_fields(items: Seq<(String, Value)>) -> bool {
        forall|i: int, j: int| 0 <= i < items.len() && 0 <= j < items.len() && i != j ==> items[i].0@ != items[j].0@
    }
    pub proof fn unique_from_map(items: Seq<(String, Value)>)
        requires items.map(|i: int, pair: (String, Value)| pair.0@).no_duplicates(),
        ensures unique_fields(items),
    {
        let names = items.map(|i: int, pair: (String, Value)| pair.0@);
        assert forall|i: int, j: int| 0 <= i < items.len() && 0 <= j < items.len() && i != j implies items[i].0@ != items[j].0@ by {
            assert(names[i] != names[j]);
        }
    }
    pub proof fn map_key_at(items: Seq<(String, Value)>, index: int)
        requires unique_fields(items), 0 <= index < items.len(),
        ensures map_pairs(items).contains_key(items[index].0), map_pairs(items)[items[index].0] == items[index].1,
        decreases items.len(),
    {
        crate::json_specs::map_pairs_unfold(items);
        if index < items.len() - 1 {
            assert(unique_fields(items.drop_last()));
            map_key_at(items.drop_last(), index);
            assert(items[index].0@ != items.last().0@);
        }
    }
    pub proof fn map_key_witness(items: Seq<(String, Value)>, key: String)
        requires map_pairs(items).contains_key(key),
        ensures exists|i: int| 0 <= i < items.len() && items[i].0 == key,
        decreases items.len(),
    {
        crate::json_specs::map_pairs_unfold(items);
        assert(items.len() > 0);
        if key == items.last().0 { assert(items[items.len() - 1].0 == key); }
        else {
            map_key_witness(items.drop_last(), key);
            let i = choose|i: int| 0 <= i < items.drop_last().len() && items.drop_last()[i].0 == key;
            assert(items[i].0 == key);
        }
    }
    pub proof fn field_at(items: Seq<(String, Value)>, index: int)
        requires unique_fields(items), 0 <= index < items.len(),
        ensures field_in(items, items[index].0@) == Some(items[index].1),
        decreases items.len(),
    {
        reveal(field_in);
        if index > 0 {
            assert(items[index].0@ != items[0].0@);
            assert(unique_fields(items.drop_first()));
            field_at(items.drop_first(), index - 1);
        }
    }
    pub proof fn field_witness(items: Seq<(String, Value)>, name: Seq<char>)
        requires field_in(items, name).is_some(),
        ensures exists|i: int| 0 <= i < items.len() && items[i].0@ == name && field_in(items, name) == Some(items[i].1),
        decreases items.len(),
    {
        reveal(field_in);
        assert(items.len() > 0);
        if items[0].0@ == name { assert(field_in(items, name) == Some(items[0].1)); }
        else {
            field_witness(items.drop_first(), name);
            let i = choose|i: int| 0 <= i < items.drop_first().len() && items.drop_first()[i].0@ == name
                && field_in(items.drop_first(), name) == Some(items.drop_first()[i].1);
            assert(items[i + 1].0@ == name && field_in(items, name) == Some(items[i + 1].1));
        }
    }
    pub open spec fn field_binding(map: Map<String, Value>, name: Seq<char>, value: Option<Value>) -> bool {
        match value {
            Some(value) => exists|key: String| map.contains_key(key) && key@ == name && map[key] == value,
            None => forall|key: String| map.contains_key(key) ==> key@ != name,
        }
    }
    pub proof fn map_field(items: Seq<(String, Value)>, name: Seq<char>)
        requires unique_fields(items),
        ensures field_binding(map_pairs(items), name, field_in(items, name)),
    {
        if field_in(items, name).is_some() {
            field_witness(items, name);
            let i = choose|i: int| 0 <= i < items.len() && items[i].0@ == name && field_in(items, name) == Some(items[i].1);
            map_key_at(items, i);
            assert(map_pairs(items).contains_key(items[i].0) && items[i].0@ == name && map_pairs(items)[items[i].0] == field_in(items, name)->Some_0);
        } else {
            assert forall|key: String| map_pairs(items).contains_key(key) implies key@ != name by {
                map_key_witness(items, key);
                let i = choose|i: int| 0 <= i < items.len() && items[i].0 == key;
                field_at(items, i);
            }
        }
    }
    pub open spec fn input_field(name: Seq<char>) -> bool {
        name == "tasks"@ || name == "commits"@ || name == "batches"@ || name == "readers"@
            || name == "consumers"@ || name == "producers"@ || name == "checkpoints"@
    }
    pub open spec fn same_inputs(before: Value, after: Value) -> bool {
        forall|name: Seq<char>| input_field(name) ==> field(before, name) == field(after, name)
    }
    pub proof fn edit_preserves_inputs(before: Value, after: Value, edit: crate::queue_metadata::Edit)
        requires crate::environment::string_content_contract(),
            crate::json_update::edited(before, after, edit.path@, edit.replacement),
            !input_field(edit.path@[0]@),
        ensures same_inputs(before, after),
    {
        let b = match before { Value::Object(map) => map, _ => arbitrary() };
        let a = match after { Value::Object(map) => map, _ => arbitrary() };
        crate::json_specs::map_entries_unique(b);
        crate::json_specs::map_entries_unique(a);
        unique_from_map(entries(b)); unique_from_map(entries(a));
        assert forall|name: Seq<char>| input_field(name) implies field(before, name) == field(after, name) by {
            map_field(entries(b), name); map_field(entries(a), name);
            let old_map = map_pairs(entries(b)); let new_map = map_pairs(entries(a));
            if field(before, name).is_some() {
                let key = choose|key: String| old_map.contains_key(key) && key@ == name && old_map[key] == field(before, name)->Some_0;
                assert(key != edit.path@[0]);
                if field(after, name).is_some() {
                    let other = choose|other: String| new_map.contains_key(other) && other@ == name && new_map[other] == field(after, name)->Some_0;
                    assert(other == key);
                }
            } else if field(after, name).is_some() {
                let key = choose|key: String| new_map.contains_key(key) && key@ == name && new_map[key] == field(after, name)->Some_0;
                assert(key != edit.path@[0]);
                assert(old_map.contains_key(key));
            }
        }
    }
    pub proof fn trace_preserves_inputs(states: Seq<Value>, edits: Seq<crate::queue_metadata::Edit>, count: nat)
        requires crate::environment::string_content_contract(), states.len() == edits.len() + 1, count <= edits.len(),
            forall|i: int| 0 <= i < edits.len() ==> crate::json_update::edited(states[i], states[i + 1], edits[i].path@, edits[i].replacement)
                && !input_field(edits[i].path@[0]@),
        ensures same_inputs(states[0], states[count as int]),
        decreases count,
    {
        if count > 0 {
            trace_preserves_inputs(states, edits, (count - 1) as nat);
            edit_preserves_inputs(states[count - 1], states[count as int], edits[count - 1]);
        }
    }
    pub proof fn retirement_preserves_inputs(before: Value, after: Value, event: Value)
        requires crate::environment::string_content_contract(),
            crate::queue_metadata::text(event, "type"@) == Some("RetentionAdvanced"@),
            crate::queue_metadata::outcome(before, after, event, Ok(())),
        ensures same_inputs(before, after),
    {
        let edits = choose|edits: Seq<crate::queue_metadata::Edit>| crate::queue_metadata::plan_matches(event, edits)
            && crate::queue_metadata::applied(before, after, edits);
        let states = crate::queue_metadata::applied_trace(before, after, edits);
        assert forall|i: int| 0 <= i < edits.len() implies !input_field(edits[i].path@[0]@) by {
            let plan = crate::queue_metadata::intended(event);
            assert(crate::queue_metadata::business(event)[0].path == seq!["retired_prefix"@]);
            assert(crate::queue_metadata::business(event)[1].path == seq!["retired_positions"@]);
            assert(crate::queue_metadata::edit_matches(edits[i], plan[i]));
            assert(plan[i].path.len() > 0);
            assert(edits[i].path@[0]@ == plan[i].path[0]);
            if i == 0 { assert(edits[i].path@[0]@ == "retired_prefix"@); }
            else if i == 1 { assert(edits[i].path@[0]@ == "retired_positions"@); }
            else { assert(i == 2); assert(edits[i].path@[0]@ == "requests"@); }
        }
        trace_preserves_inputs(states, edits, edits.len());
    }
    pub proof fn selection_survives_retirement(before:Value,after:Value,roots:Roots)
        requires roots.selected_for(before),
            forall|key:Seq<char>| input_field(key) ==> field(before,key) == field(after,key),
        ensures roots.selected_for(after),
    {
        assert(input_field("tasks"@)); assert(input_field("commits"@)); assert(input_field("batches"@));
        assert(input_field("readers"@)); assert(input_field("consumers"@)); assert(input_field("producers"@));
        assert(input_field("checkpoints"@));
        assert(crate::queue_roots::training_consumer(before) == crate::queue_roots::training_consumer(after));
        assert(crate::queue_roots::live_keys(before,roots.origin@.0,roots.origin@.1)
            == crate::queue_roots::live_keys(after,roots.origin@.0,roots.origin@.1));
    }
    pub proof fn retirement_preserves_roots(before: Value, after: Value, event: Value, progress: Value, digest: spec_fn(Seq<u8>) -> String)
        requires crate::environment::string_content_contract(),
            crate::queue_metadata::text(event, "type"@) == Some("RetentionAdvanced"@),
            crate::queue_metadata::outcome(before, after, event, Ok(())),
        ensures crate::queue_roots::live_keys(before, progress, digest) == crate::queue_roots::live_keys(after, progress, digest),
    {
        retirement_preserves_inputs(before, after, event);
        assert(input_field("tasks"@));
        assert(field(before, "tasks"@) == field(after, "tasks"@));
        assert(input_field("commits"@));
        assert(field(before, "commits"@) == field(after, "commits"@));
        assert(input_field("batches"@));
        assert(field(before, "batches"@) == field(after, "batches"@));
        assert(input_field("readers"@));
        assert(field(before, "readers"@) == field(after, "readers"@));
        assert(input_field("consumers"@));
        assert(field(before, "consumers"@) == field(after, "consumers"@));
        assert(input_field("producers"@));
        assert(field(before, "producers"@) == field(after, "producers"@));
        assert(input_field("checkpoints"@));
        assert(field(before, "checkpoints"@) == field(after, "checkpoints"@));
        assert(crate::queue_roots::training_consumer(before) == crate::queue_roots::training_consumer(after));
        assert(crate::queue_roots::training_keys(before, crate::queue_roots::training_consumer(before), progress, digest)
            == crate::queue_roots::training_keys(after, crate::queue_roots::training_consumer(after), progress, digest));
    }
}

#[cfg(all(test, not(straw_verus)))]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    fn roots() -> Roots {
        Roots {
            references: HashMap::from([("one".into(), Value::Null), ("二".into(), Value::Null)]),
            prefix: 3,
            positions: HashSet::from([4, 7]),
        }
    }
    #[test]
    fn prepared_roots_preserve_exact_owner_names() {
        let retired = Retired::fixture(roots());
        let prepared = prepare("queue:a:", retired);
        let (prefix, live) = prepared.parts();
        assert_eq!(prefix, "queue:a:");
        assert_eq!(
            live,
            HashSet::from(["queue:a:one".into(), "queue:a:二".into()])
        );
    }
    #[test]
    fn pruning_preserves_other_queues_staging_and_current_roots() {
        let owners = [
            "queue:a:old",
            "queue:a:live",
            "queue:ab:peer",
            "queue:b:peer",
            "staged:publication",
            "writer:open",
            "队列:旧",
        ]
        .into_iter()
        .map(|name| (name.to_owned(), BTreeSet::from(["pack".to_owned()])))
        .collect::<HashMap<_, _>>();
        let live = HashSet::from(["queue:a:live".to_owned()]);
        assert_eq!(
            prunable(&owners, "queue:a:", &live),
            vec!["queue:a:old".to_owned()]
        );
        assert_eq!(
            prunable(&owners, "队列:", &live),
            vec!["队列:旧".to_owned()]
        );
        assert!(prunable(&owners, "queue:absent:", &live).is_empty());
        let expected = owners
            .keys()
            .filter(|name| !live.contains(*name))
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            prunable(&owners, "", &live)
                .into_iter()
                .collect::<BTreeSet<_>>(),
            expected
        );
    }
    #[test]
    fn release_event_preserves_wire_format_and_filters_unknown_or_duplicate_names() {
        let known = HashMap::from([
            ("old".to_owned(), BTreeSet::from(["pack".to_owned()])),
            ("peer".to_owned(), BTreeSet::from(["shared".to_owned()])),
        ]);
        assert!(release_value(&known, &["absent".to_owned()]).is_none());
        assert!(release_value(&known, &[]).is_none());
        let event = release_value(
            &known,
            &["absent".to_owned(), "old".to_owned(), "old".to_owned()],
        )
        .unwrap();
        assert_eq!(
            event,
            serde_json::json!({"owners":{"old":null},"path_encoding":1})
        );
    }

    #[test]
    fn retained_check_rejects_absent_empty_and_deleted_owner_paths() {
        let mut owners = HashMap::from([("live".to_owned(), BTreeSet::from(["pack".to_owned()]))]);
        let live = HashSet::from(["live".to_owned()]);
        assert_eq!(require(&owners, &HashSet::new(), &live), Ok(()));
        assert_eq!(
            require(&owners, &HashSet::from(["pack".to_owned()]), &live),
            Err(OwnershipError::Invalid)
        );
        owners.insert("live".to_owned(), BTreeSet::new());
        assert_eq!(
            require(&owners, &HashSet::new(), &live),
            Err(OwnershipError::Invalid)
        );
        owners.remove("live");
        assert_eq!(
            require(&owners, &HashSet::new(), &live),
            Err(OwnershipError::Missing)
        );
        assert_eq!(require(&owners, &HashSet::new(), &HashSet::new()), Ok(()));
    }
}

#[cfg(straw_verus)]
verus! {
    use crate::catalog_state::{State as StorageState, Operation};
    pub open spec fn released(state: StorageState, names: Set<String>) -> StorageState {
        StorageState { owners: state.owners.remove_keys(names), ..state }
    }
    pub proof fn release_fold(state: StorageState, operations: Seq<Operation>, names: Seq<String>)
        requires operations.len() == names.len(),
            forall|i: int| 0 <= i < operations.len() ==> operations[i] == Operation::Release(names[i]),
        ensures crate::catalog_state::run(state, operations) == released(state, names.to_set()),
        decreases operations.len(),
    {
        crate::catalog_state::run_unfold(state, operations);
        if operations.len() > 0 {
            release_fold(state, operations.drop_last(), names.drop_last());
            names.drop_last().lemma_push_to_set_commute(names.last());
            assert(names.drop_last().push(names.last()) == names);
            assert(state.owners.remove_keys(names.to_set()) =~= state.owners.remove_keys(names.drop_last().to_set()).remove(names.last()));
        } else { assert(names.to_set() =~= Set::<String>::empty()); assert(released(state, names.to_set()).owners =~= state.owners); }
    }
    pub open spec fn release_event(value: Value, names: Set<String>) -> bool {
        crate::json_specs::json(value) is Object
            && field(value, "owners"@).is_some() && field(value, "owners"@)->Some_0 is Object
            && crate::json_update::fields(field(value, "owners"@)->Some_0) == Map::new(names, |key: String| Value::Null)
            && field(value, "path_encoding"@).is_some()
            && crate::json_specs::json(field(value, "path_encoding"@)->Some_0) == crate::json_specs::Json::Number(Some(1))
            && forall|key: Seq<char>| key != "owners"@ && key != "path_encoding"@ ==> field(value, key).is_none()
    }
    pub proof fn null_owner_entries(value: Value, names: Set<String>)
        requires value is Object, crate::json_update::fields(value) == Map::new(names, |key: String| Value::Null),
        ensures crate::json_specs::json(value) is Object,
            crate::json_specs::json(value)->Object_0.map(|i: int, pair: (String, Value)| pair.0).to_set() == names,
            forall|i: int| 0 <= i < crate::json_specs::json(value)->Object_0.len() ==> crate::json_specs::json(value)->Object_0[i].1 == Value::Null,
    {
        let map = match value { Value::Object(map) => map, _ => arbitrary() };
        crate::json_specs::map_entries_unique(map); unique_from_map(entries(map));
        let items = entries(map); let keys = items.map(|i: int, pair: (String, Value)| pair.0);
        assert forall|i: int| 0 <= i < items.len() implies items[i].1 == Value::Null by { map_key_at(items, i); }
        assert forall|key: String| keys.to_set().contains(key) implies names.contains(key) by {
            let i = choose|i: int| 0 <= i < keys.len() && keys[i] == key;
            map_key_at(items, i);
        }
        assert forall|key: String| names.contains(key) implies keys.to_set().contains(key) by {
            map_key_witness(items, key);
            let i = choose|i: int| 0 <= i < items.len() && items[i].0 == key;
            assert(keys[i] == key);
        }
        assert(keys.to_set() =~= names);
    }
    pub proof fn wire_releases(ns: Map<String, String>, value: Value, wire: crate::catalog_json::Wire,
        operations: Seq<Operation>, names: Set<String>, state: StorageState)
        requires field(value,"owners"@).is_some(), field(value,"owners"@)->Some_0 is Object,
            crate::json_update::fields(field(value,"owners"@)->Some_0) == Map::new(names, |key: String| Value::Null),
            field(value,"packs"@).is_none(), field(value,"deleted"@).is_none(), field(value,"reclaimed"@).is_none(),
            field(value,"staging_edges"@).is_none(), field(value,"consumed_handoffs"@).is_none(),
            crate::catalog_json::wire_matches(value, wire), crate::catalog_decode::meaning(ns, wire, operations),
        ensures crate::catalog_state::run(state, operations) == released(state, names),
    {
        use crate::catalog_decode::{Action, actions, edge_output, edge_map, edge_actions};
        null_owner_entries(field(value,"owners"@)->Some_0,names);
        let items = crate::json_specs::json(field(value,"owners"@)->Some_0)->Object_0;
        let keys = wire.owners@.map(|i: int, pair: (String, Option<Vec<String>>)| pair.0);
        assert(keys =~= items.map(|i: int, pair: (String,Value)| pair.0));
        assert forall|i: int| 0 <= i < wire.owners@.len() implies wire.owners@[i].1.is_none() by {
            assert(crate::catalog_json::owner_match(items[i].1,wire.owners@[i].1,true));
        }
        assert(wire.packs@.len() == 0 && wire.deleted@.len() == 0 && wire.reclaimed@.len() == 0 && wire.edges@.len() == 0 && wire.receipts@.len() == 0);
        let prefix = choose|prefix: Seq<Operation>| edge_output(prefix, edge_map(wire.edges@))
            && actions(operations) == actions(prefix)
                + crate::catalog_decode::owner_actions(ns, wire.owners@, crate::catalog_decode::wire_encoding(wire))
                + crate::catalog_decode::pack_actions(ns, wire.packs@, crate::catalog_decode::wire_encoding(wire))
                + crate::catalog_decode::deletion_actions(ns, wire.deleted@, crate::catalog_decode::wire_encoding(wire), false)
                + crate::catalog_decode::deletion_actions(ns, wire.reclaimed@, crate::catalog_decode::wire_encoding(wire), true)
                + seq![Action::Receipts(wire.receipts@)];
        let edges = choose|edges: Seq<(String,Vec<String>)>| edges.to_set() == edge_map(wire.edges@).kv_pairs()
            && edges.no_duplicates() && actions(prefix) == edge_actions(edges);
        assert(edge_map(wire.edges@) == Map::<String, Vec<String>>::empty());
        if edges.len() > 0 { assert(edges.to_set().contains(edges[0])); }
        assert(edges.len() == 0 && prefix.len() == 0);
        assert(operations.len() == keys.len() + 1);
        assert forall|i: int| 0 <= i < keys.len() implies operations.drop_last()[i] == Operation::Release(keys[i]) by {
            assert(actions(operations)[i] == Action::Release(keys[i]));
        }
        release_fold(state,operations.drop_last(),keys);
        crate::catalog_state::run_unfold(state,operations);
        assert(actions(operations).last() == Action::Receipts(wire.receipts@));
        assert(crate::catalog_state::run(state,operations).receipts =~= state.receipts);
    }
    pub proof fn append_releases(context: crate::catalog_frame::Context, before: crate::catalog_log::State,
        after: crate::catalog_log::State, input: Value, names: Set<String>)
        requires crate::catalog_log::valid(before), crate::catalog_log::ready(before),
            crate::catalog_log::valid(after), crate::catalog_log::ready(after),
            after.history.len() == before.history.len() + 1, after.history.drop_last() == before.history,
            release_event(input,names), crate::catalog_append::intended(context,input,before.indexes.edges,
                after.history.last().payload,after.history.last().operations),
        ensures after.indexes == released(before.indexes,names),
    {
        let (value,wire,keys) = choose|value:Value,wire:crate::catalog_json::Wire,keys:Map<String,Vec<String>>|
            crate::catalog_append::binds_run(input,value,context.run)
                && crate::json_specs::parses(after.history.last().payload,value)
                && crate::catalog_json::schema(value) && crate::catalog_json::wire_matches(value,wire)
                && crate::catalog_state::edge_view(keys) == before.indexes.edges
                && crate::catalog_decode::admitted(wire,keys)
                && crate::catalog_decode::resolved_wire(context.namespace,wire)
                && crate::catalog_decode::meaning(context.namespace,wire,after.history.last().operations);
        wire_releases(context.namespace,value,wire,after.history.last().operations,names,before.indexes);
        crate::catalog_log::complete_history(before); crate::catalog_log::complete_history(after);
        crate::catalog_log::history_unfold(after.history);
    }
    pub proof fn prune_preserves_protected(state: StorageState, removed: Set<String>, prefix: Seq<char>, live: Set<String>)
        requires forall|owner: String| removed.contains(owner) ==> byte_prefix(owner@,prefix) && !live.contains(owner),
        ensures forall|owner: String| state.owners.contains_key(owner) && (live.contains(owner) || !byte_prefix(owner@,prefix))
            ==> released(state,removed).owners.contains_key(owner) && released(state,removed).owners[owner] == state.owners[owner]
                && forall|pack: String| state.owners[owner].contains(pack) ==> !crate::catalog_state::eligible(released(state,removed),pack),
    {
        assert forall|owner: String| state.owners.contains_key(owner) && (live.contains(owner) || !byte_prefix(owner@,prefix))
            implies released(state,removed).owners.contains_key(owner) && released(state,removed).owners[owner] == state.owners[owner]
                && forall|pack: String| state.owners[owner].contains(pack) ==> !crate::catalog_state::eligible(released(state,removed),pack) by {
            assert(!removed.contains(owner));
            assert(released(state,removed).owners.contains_key(owner));
            assert(released(state,removed).owners[owner] == state.owners[owner]);
            assert forall|pack: String| state.owners[owner].contains(pack) implies !crate::catalog_state::eligible(released(state,removed),pack) by {
                assert(crate::catalog_state::rooted(released(state,removed),pack));
            }
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
    ensures result.is_none() == known@.dom().intersect(names@.to_set()).is_empty(),
        result.is_some() ==> release_event(result->Some_0,known@.dom().intersect(names@.to_set())),
))]
fn release_value(
    known: &HashMap<String, std::collections::BTreeSet<String>>,
    names: &[String],
) -> Option<Value> {
    let mut updates = serde_json::Map::new();
    let mut any = false;
    let mut i = 0usize;
    #[cfg(straw_verus)]
    proof! {
        crate::json_specs::map_pairs_unfold(entries(updates));
        assert(crate::json_specs::object(updates) == Map::<String,Value>::empty());
        assert(names@.take(0).to_set() =~= Set::<String>::empty());
        assert(Map::new(known@.dom().intersect(names@.take(0).to_set()), |key:String| Value::Null) =~= Map::<String,Value>::empty());
    }
    #[cfg_attr(straw_verus, verus_spec(
        invariant crate::environment::string_hash_contract(), crate::environment::string_order_contract(), i <= names.len(),
            crate::json_specs::object(updates) == Map::new(known@.dom().intersect(names@.take(i as int).to_set()), |key: String| Value::Null),
            any == !known@.dom().intersect(names@.take(i as int).to_set()).is_empty(),
        decreases names.len()-i,
    ))]
    while i < names.len() {
        #[cfg(straw_verus)]
        proof! {
            names@.take(i as int).lemma_push_to_set_commute(names@[i as int]);
            assert(names@.take(i as int+1) == names@.take(i as int).push(names@[i as int]));
        }
        if known.contains_key(&names[i]) {
            updates.insert(names[i].clone(), Value::Null);
            any = true;
        }
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! { assert(names@.take(i as int) == names@); }
    if !any {
        return None;
    }
    let mut object = serde_json::Map::new();
    #[cfg(straw_verus)]
    proof! { assert forall|key:Seq<char>| field_in(entries(object),key).is_none() by {}; }
    object.insert("owners".to_owned(), Value::Object(updates));
    object.insert("path_encoding".to_owned(), Value::Number(1u64.into()));
    Some(Value::Object(object))
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(resolver).valid(), crate::catalog_log::valid(old(log)@), crate::catalog_log::ready(old(log)@), old(device).valid(), !old(device)@.buffered,
        crate::environment::string_hash_contract(), crate::environment::string_order_contract(), crate::environment::string_content_contract(),
        crate::catalog_target::interpreted(crate::catalog_frame::Context {run:run@,namespace:old(resolver)@},old(log)@.history),
    ensures crate::catalog_log::at_boundary(old(log)@,old(device)@) && crate::catalog_log::ready(final(log)@)
            ==> crate::catalog_log::at_boundary(final(log)@,final(device)@),
        final(device).loan() == old(device).loan(), final(resolver).valid(), crate::catalog_log::valid(final(log)@), final(resolver)@ == old(resolver)@,
        final(device).valid(), final(device)@.identity == old(device)@.identity, !final(device)@.buffered,
        crate::catalog_target::interpreted(crate::catalog_frame::Context {run:run@,namespace:old(resolver)@},final(log)@.history),
        result.is_err() ==> final(log)@.history == old(log)@.history && final(log)@.indexes == old(log)@.indexes,
        match result { Err(crate::catalog_append::Error::Append(crate::catalog_log::AppendFailure::Write(_))) => !crate::catalog_log::ready(final(log)@), _ => true },
        result.is_ok() ==> crate::catalog_log::ready(final(log)@) && final(log)@.indexes == released(old(log)@.indexes,names@.to_set())
            && (names@.to_set().disjoint(old(log)@.indexes.owners.dom()) ==> final(log)@ == old(log)@ && final(device)@ == old(device)@)
            && (!names@.to_set().disjoint(old(log)@.indexes.owners.dom()) ==> final(device)@.durable == final(log)@.offset
                && final(device)@.cursor == final(log)@.offset && final(log)@.floor == final(log)@.offset
                && final(log)@.history.len() == old(log)@.history.len()+1 && final(log)@.history.drop_last() == old(log)@.history),
))]
pub fn release<R: crate::catalog_decode::Resolver, D: crate::wal_device::Writer>(
    log: &mut crate::catalog_log::LogState,
    resolver: &mut R,
    run: &str,
    names: &[String],
    device: &mut D,
) -> Result<(), crate::catalog_append::Error<R::Error, D::Error>> {
    let indexes = log.indexes();
    #[cfg(straw_verus)]
    proof! { assert(indexes.owners@.dom() =~= log@.indexes.owners.dom()); }
    let value = release_value(&indexes.owners, names);
    #[cfg(straw_verus)]
    proof! {
        assert(names@.to_set().intersect(log@.indexes.owners.dom()) =~= log@.indexes.owners.dom().intersect(names@.to_set()));
        assert(log@.indexes.owners.remove_keys(names@.to_set().intersect(log@.indexes.owners.dom())) =~= log@.indexes.owners.remove_keys(names@.to_set()));
    }
    let Some(value) = value else {
        #[cfg(straw_verus)]
        proof! { assert(log@.indexes.owners.remove_keys(names@.to_set()) =~= log@.indexes.owners); }
        return Ok(());
    };
    crate::catalog_append::append(log, resolver, run, value, device)?;
    #[cfg(straw_verus)]
    proof! {
        append_releases(crate::catalog_frame::Context {run:run@,namespace:resolver@},old(log)@,log@,value,
            old(log)@.indexes.owners.dom().intersect(names@.to_set()));
        assert(old(log)@.indexes.owners.remove_keys(old(log)@.indexes.owners.dom().intersect(names@.to_set()))
            =~= old(log)@.indexes.owners.remove_keys(names@.to_set()));
    }
    Ok(())
}
