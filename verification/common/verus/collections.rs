//! Trusted standard-collection primitives absent from pinned vstd.
//! Ownership iteration transfers existing entries without copying their values.
use std::collections::{HashMap, HashSet, hash_map::IntoIter, hash_set::IntoIter as SetIntoIter};
use std::alloc::Allocator;
use vstd::prelude::*;

use vstd::std_specs::hash::{obeys_key_model, builds_valid_hashers};

verus! {
    // Standard BTreeSet::from_iter semantics missing from the pinned vstd.
    #[verifier::external_body]
    pub proof fn btree_bulk<T: Ord>(remaining: Seq<T>, value: std::collections::BTreeSet<T>)
        requires vstd::laws_cmp::obeys_cmp::<T>(),
            vstd::std_specs::iter::FromIteratorSpec::from_iter_ensures(remaining, value),
        ensures value@ == remaining.to_set(),
    {}

    pub assume_specification<T>[ std::mem::replace::<T> ](destination: &mut T, source: T) -> (result: T)
        ensures result == *old(destination), *final(destination) == source;

    #[verifier::external_type_specification]
    #[verifier::external_body]
    #[verifier::accept_recursive_types(K)]
    #[verifier::accept_recursive_types(V)]
    #[verifier::reject_recursive_types(A)]
    pub struct ExIntoIter<K, V, A: Allocator>(IntoIter<K, V, A>);

    pub uninterp spec fn remaining<K, V, A: Allocator>(iter: IntoIter<K, V, A>) -> Seq<(K, V)>;

    pub assume_specification<K, V, S, A: Allocator>[ HashMap::<K, V, S, A>::into_iter ](map: HashMap<K, V, S, A>) -> (iter: IntoIter<K, V, A>)
        ensures (obeys_key_model::<K>() && builds_valid_hashers::<S>())
            ==> remaining(iter).to_set() == map@.kv_pairs() && remaining(iter).no_duplicates();

    pub assume_specification<K, V, A: Allocator>[ <IntoIter<K, V, A> as Iterator>::next ](iter: &mut IntoIter<K, V, A>) -> (result: Option<(K, V)>)
        ensures match result {
            Some(item) => remaining(*old(iter)).len() > 0 && item == remaining(*old(iter))[0]
                && remaining(*final(iter)) == remaining(*old(iter)).drop_first(),
            None => remaining(*old(iter)).len() == 0 && remaining(*final(iter)) == remaining(*old(iter)),
        };

    #[verifier::external_type_specification]
    #[verifier::external_body]
    #[verifier::accept_recursive_types(K)]
    #[verifier::reject_recursive_types(A)]
    pub struct ExSetIntoIter<K, A: Allocator>(SetIntoIter<K, A>);

    pub uninterp spec fn set_remaining<K, A: Allocator>(iter: SetIntoIter<K, A>) -> Seq<K>;

    pub assume_specification<K, S, A: Allocator>[ HashSet::<K, S, A>::into_iter ](set: HashSet<K, S, A>) -> (iter: SetIntoIter<K, A>)
        ensures (obeys_key_model::<K>() && builds_valid_hashers::<S>())
            ==> set_remaining(iter).to_set() == set@ && set_remaining(iter).no_duplicates();

    pub assume_specification<K, A: Allocator>[ <SetIntoIter<K, A> as Iterator>::next ](iter: &mut SetIntoIter<K, A>) -> (result: Option<K>)
        ensures match result {
            Some(item) => set_remaining(*old(iter)).len() > 0 && item == set_remaining(*old(iter))[0]
                && set_remaining(*final(iter)) == set_remaining(*old(iter)).drop_first(),
            None => set_remaining(*old(iter)).len() == 0 && set_remaining(*final(iter)) == set_remaining(*old(iter)),
        };
}
