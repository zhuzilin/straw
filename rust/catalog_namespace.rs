//! Production catalog namespace dispatch with explicit cache-context premises.
use crate::identities::Encoding;
use std::collections::HashMap;

/// `legacy` is the thin native binding to namespace_identity::identity.
/// Its root/iterator construction and cache-context premises remain explicit.
pub trait Source {
    type Error;

    fn legacy(
        &mut self,
        cache: &mut HashMap<String, String>,
        name: &str,
    ) -> Result<String, Self::Error>;
    fn absolute(&self, name: &str) -> bool;
    fn invalid(&self, unsupported: bool) -> Self::Error;
}

pub fn reset(cache: &mut HashMap<String, String>) {
    cache.clear();
}

pub fn resolve<S: Source>(
    source: &mut S,
    cache: &mut HashMap<String, String>,
    name: &str,
    encoding: Encoding,
) -> Result<String, S::Error> {
    match encoding {
        Encoding::Legacy => source.legacy(cache, name),
        Encoding::Resolved => {
            if !crate::namespace_identity::valid(name.as_bytes(), source.absolute(name)) {
                return Err(source.invalid(false));
            }
            Ok(name.to_owned())
        }
        Encoding::Unsupported => Err(source.invalid(true)),
    }
}

pub struct PathSource<
    B: crate::namespace_identity::Backend,
    F: crate::namespace_identity::Factory<B::Error>,
> {
    backend: B,
    factory: F,
}

impl<B: crate::namespace_identity::Backend, F: crate::namespace_identity::Factory<B::Error>>
    PathSource<B, F>
{
    pub fn new(backend: B, factory: F) -> Self {
        Self { backend, factory }
    }

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

impl<B: crate::namespace_identity::Backend, F: crate::namespace_identity::Factory<B::Error>> Source
    for PathSource<B, F>
{
    type Error = B::Error;
    fn legacy(
        &mut self,
        cache: &mut HashMap<String, String>,
        name: &str,
    ) -> Result<String, Self::Error> {
        self.identity(cache, name)
    }
    fn absolute(&self, name: &str) -> bool {
        self.factory.absolute(name)
    }
    fn invalid(&self, unsupported: bool) -> Self::Error {
        self.factory.catalog_error(unsupported)
    }
}

pub struct Adapter<S: Source> {
    source: S,
    cache: HashMap<String, String>,
}

impl<S: Source> Adapter<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            cache: HashMap::new(),
        }
    }
}

impl<S: Source> crate::catalog_decode::Resolver for Adapter<S> {
    type Error = S::Error;
    fn reset(&mut self) {
        reset(&mut self.cache);
    }
    fn resolve(&mut self, name: &str, encoding: Encoding) -> Result<String, Self::Error> {
        resolve(&mut self.source, &mut self.cache, name, encoding)
    }
}

impl<S: Source> crate::catalog_decode::Resolver for &mut Adapter<S> {
    type Error = S::Error;
    fn reset(&mut self) {
        crate::catalog_decode::Resolver::reset(*self);
    }
    fn resolve(&mut self, name: &str, encoding: Encoding) -> Result<String, Self::Error> {
        crate::catalog_decode::Resolver::resolve(*self, name, encoding)
    }
}

impl<B: crate::namespace_identity::Backend, F: crate::namespace_identity::Factory<B::Error>>
    Adapter<PathSource<B, F>>
{
    pub fn identity(&mut self, name: &str) -> Result<String, B::Error> {
        self.source.identity(&mut self.cache, name)
    }
}
