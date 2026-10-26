//! Resolve one path component using only filesystem and Path primitives.
#[cfg(straw_verus)]
use crate::namespace_target::{absolute, joined};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg(straw_verus)]
verus! {
    pub trait Model: crate::namespace_target::Model {
        type Error;
        spec fn names(context: Self::Context) -> Map<String, String>;
        spec fn at(context: Self::Context, path: Self::Path, prefix: String) -> bool;
        spec fn pending(context: Self::Context, base: Self::Path, candidate: Self::Path, prefix: String) -> bool;
        spec fn resolved(context: Self::Context, prefix: String) -> crate::namespace_target::Key;
    }
}
#[cfg(not(straw_verus))]
pub trait Model: crate::namespace_target::Model {
    type Error;
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Primitives: Model + crate::namespace_target::Backend {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires prefix@.len() == 0,
        ensures Self::at(self.context(), result, *prefix),
    ))]
    #[allow(
        clippy::ptr_arg,
        reason = "The verified namespace relation uses the borrowed String as its exact key."
    )]
    fn root(&self, prefix: &String) -> Self::Path;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires Self::names(self.context()).contains_key(*prefix), Self::names(self.context())[*prefix] == *identity,
        ensures Self::at(self.context(), result, *prefix),
    ))]
    #[allow(
        clippy::ptr_arg,
        reason = "The verified namespace relation uses borrowed Strings as exact keys."
    )]
    fn cached(&self, prefix: &String, identity: &String) -> Self::Path;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires !component@.contains('/'), exists|parent: String| Self::at(self.context(), *base, parent)
            && (prefix@ == parent@ + seq!['/'] + component@ || (parent@.len() == 0 && prefix@ == component@)),
        ensures Self::pending(self.context(), *base, result, *prefix),
    ))]
    #[allow(
        clippy::ptr_arg,
        reason = "The verified namespace relation uses the borrowed String as its exact key."
    )]
    fn child(&self, base: &Self::Path, component: &str, prefix: &String) -> Self::Path;
    // `pending` binds the normalized parent, its child and the original name.
    // This is the filesystem's stable namespace law for this readlink call.
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires Self::pending(old(self).context(), *base, *candidate, *prefix),
        ensures final(self)@ == old(self)@, final(self).context() == old(self).context(),
            match result {
                Ok(target) => if absolute(Self::key(target)) {
                    old(self)@[Self::key(target)] == Self::resolved(old(self).context(), *prefix)
                } else {
                    old(self)@[joined(Self::key(*base), Self::key(target))] == Self::resolved(old(self).context(), *prefix)
                },
                Err(error) => (Self::error(error) == crate::namespace_target::ErrorKind::Missing
                    || Self::error(error) == crate::namespace_target::ErrorKind::NotLink)
                    ==> Self::key(*candidate) == Self::resolved(old(self).context(), *prefix),
            },
    ))]
    #[allow(
        clippy::ptr_arg,
        reason = "The verified namespace relation uses the borrowed String as its exact key."
    )]
    fn child_link(
        &mut self,
        base: &Self::Path,
        candidate: &Self::Path,
        prefix: &String,
    ) -> Result<Self::Path, Self::IoError>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures Self::key(result) == joined(Self::key(*base), Self::key(target)),
    ))]
    fn join(&self, base: &Self::Path, target: Self::Path) -> Self::Path;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires Self::key(*path) == Self::resolved(self.context(), *prefix),
        ensures result.is_ok() ==> Self::names(self.context()).contains_key(*prefix)
            && result->Ok_0 == Self::names(self.context())[*prefix]
            && Self::at(self.context(), *path, *prefix),
    ))]
    #[allow(
        clippy::ptr_arg,
        reason = "The verified namespace relation uses the borrowed String as its exact key."
    )]
    fn relative(&self, path: &Self::Path, prefix: &String) -> Result<String, Self::Error>;
    fn io_error(&self, error: Self::IoError) -> Self::Error;
    fn target_error(&self, error: crate::namespace_target::Error<Self::IoError>) -> Self::Error;
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Backend<P: Primitives> {
    primitives: P,
}
#[cfg(straw_verus)]
verus! {
    impl<P: Primitives> View for Backend<P> {
        type V = Map<String, String>;
        closed spec fn view(&self) -> Self::V { P::names(self.primitives.context()) }
    }
    impl<P: Primitives> crate::namespace_identity::BackendModel for Backend<P> {
        type Position = P::Path;
        closed spec fn at(&self, position: Self::Position, prefix: String) -> bool {
            P::at(self.primitives.context(), position, prefix)
        }
    }
}
#[cfg(not(straw_verus))]
impl<P: Primitives> crate::namespace_identity::BackendModel for Backend<P> {
    type Position = P::Path;
}
#[cfg_attr(straw_verus, verus_verify)]
impl<P: Primitives> Backend<P> {
    #[cfg_attr(straw_verus, verus_spec(result => ensures result@ == P::names(primitives.context())))]
    pub fn new(primitives: P) -> Self {
        Self { primitives }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<P: Primitives> crate::namespace_identity::Backend for Backend<P> {
    type Error = P::Error;
    fn root(&self, prefix: &String) -> Self::Position {
        self.primitives.root(prefix)
    }
    fn from_cache(&self, prefix: &String, identity: &String) -> Self::Position {
        self.primitives.cached(prefix, identity)
    }
    fn step(
        &mut self,
        position: Self::Position,
        component: &str,
        prefix: &String,
    ) -> Result<(Self::Position, String), Self::Error> {
        let candidate = self.primitives.child(&position, component, prefix);
        let resolved = match self.primitives.child_link(&position, &candidate, prefix) {
            Ok(target) => {
                let path = if self.primitives.is_absolute(&target) {
                    target
                } else {
                    self.primitives.join(&position, target)
                };
                match crate::namespace_target::resolve(&mut self.primitives, path) {
                    Ok(resolved) => resolved,
                    Err(error) => return Err(self.primitives.target_error(error)),
                }
            }
            Err(error) => {
                if matches!(
                    self.primitives.classify(&error),
                    crate::namespace_target::ErrorKind::Missing
                        | crate::namespace_target::ErrorKind::NotLink
                ) {
                    candidate
                } else {
                    return Err(self.primitives.io_error(error));
                }
            }
        };
        let identity = self.primitives.relative(&resolved, prefix)?;
        Ok((resolved, identity))
    }
}
