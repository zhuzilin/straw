//! Production catalog namespace dispatch with explicit cache-context premises.
use crate::identities::Encoding;
use std::collections::{BTreeSet, HashMap};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub struct NativeKeySet;
#[cfg_attr(straw_verus, verus_verify)]
impl crate::catalog_decode::KeySet for NativeKeySet {
    fn from_values(values: Vec<String>) -> BTreeSet<String> {
        #[cfg(straw_verus)]
        let original = verus_exec_expr!(Ghost(values@));
        let iter = values.into_iter();
        #[cfg(straw_verus)]
        proof! { vstd::std_specs::iter::axiom_from_iterator_ensures(iter); }
        let result = BTreeSet::from_iter(iter);
        #[cfg(straw_verus)]
        proof! { crate::collection_specs::btree_bulk(original@, result); }
        result
    }
}

#[cfg(straw_verus)]
verus! { pub trait SourceModel: View<V = Map<String, String>> {} }
#[cfg(not(straw_verus))]
pub trait SourceModel {}
#[cfg(not(straw_verus))]
impl<T> SourceModel for T {}

/// `legacy` is the thin native binding to namespace_identity::identity.
/// Its root/iterator construction and cache-context premises remain explicit.
#[cfg_attr(straw_verus, verus_verify)]
pub trait Source: SourceModel {
    type Error;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::environment::string_hash_contract(), crate::environment::string_content_contract(),
            crate::namespace_identity::borrowed_strings(), crate::namespace_identity::cached(old(self)@, old(cache)@),
        ensures final(self)@ == old(self)@, crate::namespace_identity::cached(final(self)@, final(cache)@),
            result.is_ok() ==> old(self)@.contains_key(*name) && result->Ok_0 == old(self)@[*name],
    ))]
    #[allow(
        clippy::ptr_arg,
        reason = "The verified namespace map uses the borrowed String as its exact key."
    )]
    fn legacy(
        &mut self,
        cache: &mut HashMap<String, String>,
        name: &String,
    ) -> Result<String, Self::Error>;
    fn absolute(&self, name: &str) -> bool;
    fn invalid(&self, unsupported: bool) -> Self::Error;
}

#[cfg_attr(straw_verus, verus_spec(ensures final(cache)@ == Map::<String, String>::empty()))]
pub fn reset(cache: &mut HashMap<String, String>) {
    cache.clear();
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::environment::string_hash_contract(), crate::environment::string_content_contract(),
        crate::namespace_identity::borrowed_strings(), crate::namespace_identity::cached(old(source)@, old(cache)@),
    ensures final(source)@ == old(source)@, crate::namespace_identity::cached(final(source)@, final(cache)@),
        result.is_ok() ==> crate::catalog_decode::resolves(old(source)@, *name, encoding)
            && result->Ok_0 == crate::catalog_decode::identity(old(source)@, *name, encoding),
))]
pub fn resolve<S: Source>(
    source: &mut S,
    cache: &mut HashMap<String, String>,
    name: &String,
    encoding: Encoding,
) -> Result<String, S::Error> {
    match encoding {
        Encoding::Legacy => source.legacy(cache, name),
        Encoding::Resolved => {
            if !crate::namespace_identity::valid(
                name.as_str().as_bytes(),
                source.absolute(name.as_str()),
            ) {
                return Err(source.invalid(false));
            }
            Ok(name.clone())
        }
        Encoding::Unsupported => Err(source.invalid(true)),
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct PathSource<
    B: crate::namespace_identity::Backend,
    F: crate::namespace_identity::Factory<B::Error>,
> {
    backend: B,
    factory: F,
}
#[cfg(straw_verus)]
verus! {
    impl<B: crate::namespace_identity::Backend, F: crate::namespace_identity::Factory<B::Error>> View for PathSource<B, F> {
        type V = Map<String, String>;
        closed spec fn view(&self) -> Self::V { self.backend@ }
    }
    impl<B: crate::namespace_identity::Backend, F: crate::namespace_identity::Factory<B::Error>> SourceModel for PathSource<B, F> {}
}
#[cfg_attr(straw_verus, verus_verify)]
impl<B: crate::namespace_identity::Backend, F: crate::namespace_identity::Factory<B::Error>>
    PathSource<B, F>
{
    #[cfg_attr(straw_verus, verus_spec(result => ensures result@ == backend@))]
    pub fn new(backend: B, factory: F) -> Self {
        Self { backend, factory }
    }
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::environment::string_hash_contract(), crate::environment::string_content_contract(),
            crate::namespace_identity::borrowed_strings(), crate::namespace_identity::cached(old(self)@, old(cache)@),
        ensures final(self)@ == old(self)@, crate::namespace_identity::cached(final(self)@, final(cache)@),
            result.is_ok() ==> (exists|key: String| key@ == name@ && old(self)@.contains_key(key) && result->Ok_0 == old(self)@[key])
                && result->Ok_0@.len() > 0,
    ))]
    pub fn identity(
        &mut self,
        cache: &mut HashMap<String, String>,
        name: &str,
    ) -> Result<String, B::Error> {
        let mut parts = self.factory.parts(name);
        let absolute = self.factory.absolute(name);
        match crate::namespace_identity::identity(
            &mut self.backend,
            cache,
            name,
            absolute,
            &mut parts,
        ) {
            Ok(identity) => Ok(identity),
            Err(error) => Err(self.factory.identity_error(error)),
        }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<B: crate::namespace_identity::Backend, F: crate::namespace_identity::Factory<B::Error>> Source
    for PathSource<B, F>
{
    type Error = B::Error;
    fn legacy(
        &mut self,
        cache: &mut HashMap<String, String>,
        name: &String,
    ) -> Result<String, Self::Error> {
        let result = self.identity(cache, name.as_str());
        #[cfg(straw_verus)]
        proof! {
            if result.is_ok() {
                let key = choose|key: String| key@ == name@ && self@.contains_key(key) && result->Ok_0 == self@[key];
                assert(key == *name);
            }
        }
        result
    }
    fn absolute(&self, name: &str) -> bool {
        self.factory.absolute(name)
    }
    fn invalid(&self, unsupported: bool) -> Self::Error {
        self.factory.catalog_error(unsupported)
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Adapter<S: Source> {
    source: S,
    cache: HashMap<String, String>,
}
#[cfg(straw_verus)]
verus! {
    impl<S: Source> View for Adapter<S> {
        type V = Map<String, String>;
        closed spec fn view(&self) -> Self::V { self.source@ }
    }
    impl<S: Source> crate::catalog_decode::ResolverModel for Adapter<S> {
        closed spec fn valid(&self) -> bool {
            crate::environment::string_hash_contract() && crate::environment::string_content_contract()
                && crate::namespace_identity::borrowed_strings()
                && crate::namespace_identity::cached(self.source@, self.cache@)
        }
    }
    impl<S: Source> View for &mut Adapter<S> {
        type V = Map<String, String>;
        closed spec fn view(&self) -> Self::V { (&**self).view() }
    }
    impl<S: Source> crate::catalog_decode::ResolverModel for &mut Adapter<S> {
        closed spec fn valid(&self) -> bool { crate::catalog_decode::ResolverModel::valid(&**self) }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<S: Source> Adapter<S> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::environment::string_hash_contract(), crate::environment::string_content_contract(),
            crate::namespace_identity::borrowed_strings(),
        ensures crate::catalog_decode::ResolverModel::valid(&result), result@ == source@,
    ))]
    pub fn new(source: S) -> Self {
        Self {
            source,
            cache: HashMap::new(),
        }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<S: Source> crate::catalog_decode::Resolver for Adapter<S> {
    type Error = S::Error;
    type Keys = NativeKeySet;
    fn reset(&mut self) {
        reset(&mut self.cache);
    }
    fn resolve(&mut self, name: &String, encoding: Encoding) -> Result<String, Self::Error> {
        resolve(&mut self.source, &mut self.cache, name, encoding)
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<S: Source> crate::catalog_decode::Resolver for &mut Adapter<S> {
    type Error = S::Error;
    type Keys = NativeKeySet;
    fn reset(&mut self) {
        crate::catalog_decode::Resolver::reset(*self);
    }
    fn resolve(&mut self, name: &String, encoding: Encoding) -> Result<String, Self::Error> {
        crate::catalog_decode::Resolver::resolve(*self, name, encoding)
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<B: crate::namespace_identity::Backend, F: crate::namespace_identity::Factory<B::Error>>
    Adapter<PathSource<B, F>>
{
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::catalog_decode::ResolverModel::valid(&*old(self)),
        ensures crate::catalog_decode::ResolverModel::valid(&*final(self)), (*final(self))@ == (*old(self))@,
            result.is_ok() ==> (exists|key: String| key@ == name@ && old(self)@.contains_key(key) && result->Ok_0 == old(self)@[key])
                && result->Ok_0@.len() > 0,
    ))]
    pub fn identity(&mut self, name: &str) -> Result<String, B::Error> {
        self.source.identity(&mut self.cache, name)
    }
}
