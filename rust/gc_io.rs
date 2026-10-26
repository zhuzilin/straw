//! Physical deletion dispatch used by the production GC batch adapter.
//! Metadata is advisory; only unlink and directory barriers establish absence.
use crate::gc_batch::Removal;

pub trait IoError {
    fn is_missing(&self) -> bool;
}

/// Only NotFound becomes an absent name/directory; other I/O errors propagate.
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

pub trait Model: crate::gc_batch::BatchModel + Sized {
    type Path;
    type Directory;
}

pub trait Primitives: Model {
    type Error;

    fn resolve(&self, index: usize) -> Result<Self::Path, Self::Error>;

    fn parent(&self, index: usize, path: &Self::Path) -> Self::Parent;

    fn point(&mut self, point: &str) -> Result<(), Self::Error>;

    // A missing/stale stat result deliberately gives no absence guarantee.

    fn stat(&mut self, index: usize, path: &Self::Path) -> Result<Option<u64>, Self::Error>;

    // Both successful unlink and NotFound confirm current absence. An error
    // can still remove a name, but permits no receipt without later barriers.

    fn remove(&mut self, index: usize, path: &Self::Path) -> Result<bool, Self::Error>;

    fn open_parent(
        &mut self,
        parent: &Self::Parent,
    ) -> Result<Option<Self::Directory>, Self::Error>;

    fn sync(&mut self, directory: &Self::Directory) -> Result<(), Self::Error>;

    // The caller must supply a repair implementation that syncs all ancestors;
    // merely creating an empty parent does not satisfy this contract.

    fn repair(&mut self, parent: &Self::Parent) -> Result<(), Self::Error>;
}

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
