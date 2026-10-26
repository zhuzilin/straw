//! Namespace identity cache and the actual ordered component resolver.
use std::collections::HashMap;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error<E> {
    Invalid,
    Pool,
    Changed,
    Read(E),
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn next(mode: u8, byte: u8) -> Option<u8> {
        if byte == 92 { None }
        else if byte == 47 { if mode == 3 { Some(0) } else { None } }
        else if byte == 46 { Some(if mode == 0 { 1 } else if mode == 1 { 2 } else { 3 }) }
        else { Some(3) }
    }
    pub open spec fn scanned(bytes: Seq<u8>) -> Option<u8>
        decreases bytes.len(),
    {
        if bytes.len() == 0 { Some(0) }
        else { match scanned(bytes.drop_last()) { None => None, Some(mode) => next(mode, bytes.last()) } }
    }
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == next(mode, byte)))]
fn advance(mode: u8, byte: u8) -> Option<u8> {
    if byte == b'\\' {
        None
    } else if byte == b'/' {
        if mode == 3 { Some(0) } else { None }
    } else if byte == b'.' {
        Some(if mode == 0 {
            1
        } else if mode == 1 {
            2
        } else {
            3
        })
    } else {
        Some(3)
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (!absolute && scanned(bytes@) == Some(3)),
))]
pub fn valid(bytes: &[u8], absolute: bool) -> bool {
    if absolute {
        return false;
    }
    let mut mode = 0u8;
    let mut index = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant index <= bytes.len(), !absolute, scanned(bytes@.take(index as int)) == Some(mode),
        decreases bytes.len() - index,
    ))]
    while index < bytes.len() {
        #[cfg(straw_verus)]
        proof! {
            assert(bytes@.take(index as int + 1).drop_last() == bytes@.take(index as int));
            assert(bytes@.take(index as int + 1).last() == bytes@[index as int]);
        }
        let Some(next) = advance(mode, bytes[index]) else {
            #[cfg(straw_verus)]
            proof! {
                assert(scanned(bytes@.take(index as int + 1)).is_none());
                failed_suffix(bytes@, index as int + 1);
            }
            return false;
        };
        mode = next;
        index += 1;
    }
    #[cfg(straw_verus)]
    proof! { assert(bytes@.take(index as int) == bytes@); }
    mode == 3
}

#[cfg(straw_verus)]
verus! {
    pub proof fn failed_suffix(bytes: Seq<u8>, length: int)
        requires 0 <= length <= bytes.len(), scanned(bytes.take(length)).is_none(),
        ensures scanned(bytes).is_none(),
        decreases bytes.len() - length,
    {
        if length < bytes.len() {
            assert(bytes.take(length + 1).drop_last() == bytes.take(length));
            failed_suffix(bytes, length + 1);
        } else {
            assert(bytes.take(length) == bytes);
        }
    }
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn join(parts: Seq<Seq<char>>) -> Seq<char>
        decreases parts.len(),
    {
        if parts.len() == 0 { Seq::empty() }
        else if parts.len() == 1 { parts[0] }
        else { join(parts.drop_last()) + seq!['/'] + parts.last() }
    }
    pub open spec fn components(parts: Seq<Seq<char>>) -> bool {
        forall|i: int| 0 <= i < parts.len() ==> !parts[i].contains('/')
    }
    pub trait PartsModel: View<V = Seq<Seq<char>>> {}
    pub trait BackendModel: View<V = Map<String, String>> {
        type Position;
        spec fn at(&self, position: Self::Position, prefix: String) -> bool;
    }
    pub open spec fn cached(namespace: Map<String, String>, cache: Map<String, String>) -> bool {
        cache.dom().subset_of(namespace.dom())
            && forall|key: String| cache.contains_key(key) ==> cache[key] == namespace[key]
    }
    // The pinned vstd has an opaque relation for borrowed HashMap keys. This
    // standard String/str relation preserves the native allocation-free lookup.
    pub open spec fn borrowed_strings() -> bool {
        forall|cache: Map<String, String>, name: &str, value: String|
            vstd::std_specs::hash::maps_borrowed_key_to_value(cache, name, value) ==
                (exists|key: String| cache.contains_key(key) && key@ == name@ && cache[key] == value)
    }
}
#[cfg(not(straw_verus))]
pub trait PartsModel {}
#[cfg(not(straw_verus))]
impl<T> PartsModel for T {}
#[cfg(not(straw_verus))]
pub trait BackendModel {
    type Position;
}

/// The native adapter is a borrowed str::split('/'), with no copied components.
#[cfg_attr(straw_verus, verus_verify)]
pub trait Parts: PartsModel {
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures match result {
            Some(part) => old(self)@.len() > 0 && part@ == old(self)@[0] && final(self)@ == old(self)@.drop_first(),
            None => old(self)@.len() == 0 && final(self)@ == old(self)@,
        },
    ))]
    fn next(&mut self) -> Option<&str>;
}

/// Per-component filesystem/path effects, independent of the identity cache.
/// The implementation must bind each Position to the original prefix even
/// when several original names refer to the same canonical destination.
#[cfg_attr(straw_verus, verus_verify)]
pub trait Backend: BackendModel {
    type Error;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires prefix@.len() == 0,
        ensures self.at(result, *prefix),
    ))]
    #[allow(clippy::ptr_arg)]
    fn root(&self, prefix: &String) -> Self::Position;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self@.contains_key(*prefix), self@[*prefix] == *identity,
        ensures self.at(result, *prefix),
    ))]
    #[allow(
        clippy::ptr_arg,
        clippy::wrong_self_convention,
        reason = "Reconstruct a position in this backend from exact verified String keys."
    )]
    fn from_cache(&self, prefix: &String, identity: &String) -> Self::Position;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires !component@.contains('/'), exists|parent: String| self.at(position, parent)
            && (prefix@ == parent@ + seq!['/'] + component@ || (parent@.len() == 0 && prefix@ == component@)),
        ensures final(self)@ == old(self)@,
            result.is_ok() ==> final(self)@.contains_key(*prefix)
                && result->Ok_0.1 == final(self)@[*prefix]
                && final(self).at(result->Ok_0.0, *prefix),
    ))]
    #[allow(clippy::ptr_arg)]
    fn step(
        &mut self,
        position: Self::Position,
        component: &str,
        prefix: &String,
    ) -> Result<(Self::Position, String), Self::Error>;
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(), crate::environment::string_content_contract(), borrowed_strings(),
        cached(old(source)@, old(cache)@), old(parts)@.len() > 0, components(old(parts)@), join(old(parts)@) == name@,
    ensures final(source)@ == old(source)@, cached(final(source)@, final(cache)@),
        old(cache)@.dom().subset_of(final(cache)@.dom()),
        forall|key: String| old(cache)@.contains_key(key) ==> final(cache)@[key] == old(cache)@[key],
        result.is_ok() ==> (exists|key: String| key@ == name@ && old(source)@.contains_key(key) && result->Ok_0 == old(source)@[key]) && result->Ok_0@.len() > 0,
))]
pub fn identity<S: Backend, P: Parts>(
    source: &mut S,
    cache: &mut HashMap<String, String>,
    name: &str,
    absolute: bool,
    parts: &mut P,
) -> Result<String, Error<S::Error>> {
    if !valid(name.as_bytes(), absolute) {
        return Err(Error::Invalid);
    }
    if let Some(identity) = cache.get(name) {
        return if identity.is_empty() {
            Err(Error::Pool)
        } else {
            Ok(identity.clone())
        };
    }
    let mut key = String::new();
    let mut position = source.root(&key);
    let mut identity = String::new();
    let mut first = true;
    #[cfg(straw_verus)]
    let original = verus_exec_expr!(Ghost(parts@));
    #[cfg(straw_verus)]
    let mut count = verus_exec_expr!(Ghost(0int));
    #[cfg_attr(straw_verus, verus_spec(
        invariant source@ == old(source)@,
            crate::environment::string_hash_contract(), crate::environment::string_content_contract(),
            cached(source@, cache@), old(cache)@.dom().subset_of(cache@.dom()),
            forall|key: String| old(cache)@.contains_key(key) ==> cache@[key] == old(cache)@[key],
            original@.len() > 0, components(original@), join(original@) == name@,
            0 <= count@ <= original@.len(), parts@ == original@.subrange(count@, original@.len() as int),
            first == (count@ == 0), key@ == join(original@.take(count@)),
            source.at(position, key),
            count@ > 0 ==> source@.contains_key(key) && identity == source@[key],
        decreases parts@.len(),
    ))]
    loop {
        let Some(component) = parts.next() else {
            #[cfg(straw_verus)]
            proof! {
                assert(count@ == original@.len());
                assert(original@.take(count@) =~= original@);
                assert(key@ == name@);
            }
            if identity.is_empty() {
                return Err(Error::Pool);
            }
            return Ok(identity);
        };
        #[cfg(straw_verus)]
        let previous = verus_exec_expr!(Ghost(key));
        if !first {
            key.push('/');
        }
        key.push_str(component);
        first = false;
        #[cfg(straw_verus)]
        proof! {
            assert(component@ == original@[count@]);
            assert(original@.take(count@ + 1).drop_last() == original@.take(count@));
            assert(original@.take(count@ + 1).last() == component@);
            assert(key@ == join(original@.take(count@ + 1)));
            assert(exists|parent: String| source.at(position, parent)
                && (key@ == parent@ + seq!['/'] + component@ || (parent@.len() == 0 && key@ == component@)));
        }
        if let Some(found) = crate::identities::lookup(cache, &key) {
            position = source.from_cache(&key, &found);
            identity = found;
        } else {
            let (resolved, found) = match source.step(position, component, &key) {
                Ok(result) => result,
                Err(error) => return Err(Error::Read(error)),
            };
            position = resolved;
            identity = found;
            if !crate::identities::remember(cache, key.clone(), identity.clone()) {
                return Err(Error::Changed);
            }
        }
        #[cfg(straw_verus)]
        proof! { let next_count = count@ + 1; *count.borrow_mut() = next_count; }
    }
}

/// Standard borrowed splitting, absolute-path classification, and error values.
#[cfg_attr(straw_verus, verus_verify)]
pub trait Factory<E> {
    type Iter<'a>: Parts;
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures result@.len() > 0, components(result@), join(result@) == name@,
    ))]
    fn parts<'a>(&self, name: &'a str) -> Self::Iter<'a>;
    fn absolute(&self, name: &str) -> bool;
    fn identity_error(&self, error: Error<E>) -> E;
    fn catalog_error(&self, unsupported: bool) -> E;
}
