//! Physical deletion dispatch used by the production GC batch adapter.
//! Metadata is advisory; only unlink and directory barriers establish absence.
use crate::gc_batch::Removal;
#[cfg(straw_verus)]
use crate::gc_batch::{indices, same_batch, under, valid};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg(straw_verus)]
verus! {
    pub trait ErrorModel { spec fn missing(&self) -> bool; }
    pub trait DirectoryModel<P>: View<V = P> {}
    pub trait Model: crate::gc_batch::BatchModel + Sized {
        type Path;
        type Directory: DirectoryModel<Self::Parent>;
        spec fn paths(&self) -> Map<nat, Self::Path>;
    }
    #[verifier::prophetic]
    pub open spec fn preserved<E: Model>(a: E, b: E) -> bool {
        b.context_valid() && b.context() == a.context() && valid(b@) && same_batch(a@, b@) && a.paths() == b.paths()
            && a@.receipted == b@.receipted
            && a@.durable_absent.subset_of(b@.durable_absent)
    }
    pub open spec fn bound<E: Model>(e: E, index: usize, path: E::Path) -> bool {
        index < e@.parents.len() && e.paths().contains_key(index as nat)
            && e.paths()[index as nat] == path
    }
}
#[cfg(not(straw_verus))]
pub trait ErrorModel {}
#[cfg(not(straw_verus))]
impl<T> ErrorModel for T {}

#[cfg_attr(straw_verus, verus_verify)]
pub trait IoError: ErrorModel {
    #[cfg_attr(straw_verus, verus_spec(result => ensures result == self.missing()))]
    fn is_missing(&self) -> bool;
}

/// Only NotFound becomes an absent name/directory; other I/O errors propagate.
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures match outcome {
        Ok(value) => result == Ok(Some(value)),
        Err(error) => if error.missing() { result == Ok(None) } else { result == Err(error) },
    },
))]
pub fn classify<T, E: IoError>(outcome: Result<T, E>) -> Result<Option<T>, E> {
    match outcome {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            if error.is_missing() {
                Ok(None)
            } else {
                Err(error)
            }
        }
    }
}

#[cfg(not(straw_verus))]
pub trait Model: crate::gc_batch::BatchModel + Sized {
    type Path;
    type Directory;
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Primitives: Model {
    type Error;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self.context_valid(), index < self@.parents.len(),
        ensures result.is_ok() ==> bound(*self, index, result->Ok_0),
    ))]
    fn resolve(&self, index: usize) -> Result<Self::Path, Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self.context_valid(), bound(*self, index, *path),
        ensures result == self@.parents[index as int],
    ))]
    fn parent(&self, index: usize, path: &Self::Path) -> Self::Parent;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).context_valid(), valid(old(self)@),
        ensures preserved(*old(self), *final(self)),
            result.is_ok() ==> final(self)@ == old(self)@,
    ))]
    fn point(&mut self, point: &str) -> Result<(), Self::Error>;

    // A missing/stale stat result deliberately gives no absence guarantee.
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).context_valid(), valid(old(self)@), bound(*old(self), index, *path),
        ensures preserved(*old(self), *final(self)),
            result.is_ok() ==> final(self)@ == old(self)@,
    ))]
    fn stat(&mut self, index: usize, path: &Self::Path) -> Result<Option<u64>, Self::Error>;

    // Both successful unlink and NotFound confirm current absence. An error
    // can still remove a name, but permits no receipt without later barriers.
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).context_valid(), valid(old(self)@), bound(*old(self), index, *path),
            old(self)@.marked.contains(index as nat),
        ensures preserved(*old(self), *final(self)),
            result.is_ok() ==> old(self)@.absent.subset_of(final(self)@.absent)
                && final(self)@.absent.contains(index as nat),
    ))]
    fn remove(&mut self, index: usize, path: &Self::Path) -> Result<bool, Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).context_valid(), valid(old(self)@),
        ensures preserved(*old(self), *final(self)),
            result.is_ok() ==> final(self)@ == old(self)@
                && (result->Ok_0.is_some() ==> result->Ok_0->Some_0@ == *parent),
    ))]
    fn open_parent(
        &mut self,
        parent: &Self::Parent,
    ) -> Result<Option<Self::Directory>, Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).context_valid(), valid(old(self)@), old(self)@.absent == indices(old(self)@),
        ensures preserved(*old(self), *final(self)),
            result.is_ok() ==> final(self)@.absent == old(self)@.absent
                && under(old(self)@, Set::empty().insert(directory@)).subset_of(final(self)@.durable_absent),
    ))]
    fn sync(&mut self, directory: &Self::Directory) -> Result<(), Self::Error>;

    // The caller must supply a repair implementation that syncs all ancestors;
    // merely creating an empty parent does not satisfy this contract.
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).context_valid(), valid(old(self)@), old(self)@.absent == indices(old(self)@),
        ensures preserved(*old(self), *final(self)),
            result.is_ok() ==> final(self)@.absent == old(self)@.absent
                && under(old(self)@, Set::empty().insert(*parent)).subset_of(final(self)@.durable_absent),
    ))]
    fn repair(&mut self, parent: &Self::Parent) -> Result<(), Self::Error>;
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(effects).context_valid(), valid(old(effects)@), index < old(effects)@.parents.len(),
        old(effects)@.marked.contains(index as nat),
    ensures preserved(*old(effects), *final(effects)),
        result.is_ok() ==> old(effects)@.absent.subset_of(final(effects)@.absent)
            && final(effects)@.absent.contains(index as nat)
            && result->Ok_0.parent == old(effects)@.parents[index as int],
))]
pub fn unlink<E: Primitives>(
    effects: &mut E,
    index: usize,
) -> Result<Removal<E::Parent>, E::Error> {
    let path = effects.resolve(index)?;
    effects.point("before_gc_stat")?;
    let metadata = effects.stat(index, &path)?;
    effects.point("before_gc_unlink")?;
    let removed = effects.remove(index, &path)?;
    let bytes = if removed {
        Some(metadata.unwrap_or(0))
    } else {
        None
    };
    Ok(Removal {
        bytes,
        parent: effects.parent(index, &path),
    })
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(effects).context_valid(), valid(old(effects)@), old(effects)@.absent == indices(old(effects)@),
    ensures preserved(*old(effects), *final(effects)),
        result.is_ok() ==> final(effects)@.absent == old(effects)@.absent
            && under(old(effects)@, Set::empty().insert(*parent)).subset_of(final(effects)@.durable_absent),
))]
pub fn sync_parent<E: Primitives>(effects: &mut E, parent: &E::Parent) -> Result<(), E::Error> {
    effects.point("before_gc_directory_sync")?;
    match effects.open_parent(parent)? {
        Some(directory) => effects.sync(&directory)?,
        None => {
            effects.point("before_gc_parent_repair")?;
            effects.repair(parent)?;
            effects.point("after_gc_parent_repair")?;
        }
    }
    Ok(())
}
