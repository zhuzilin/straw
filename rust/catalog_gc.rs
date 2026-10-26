//! Internal queue collection implements the checked transaction callback.
use crate::catalog_namespace::Source;

use crate::catalog_runtime::{Call, Catalog};

pub trait Library: crate::catalog_runtime::Library {
    fn queue_error(
        error: crate::gc_collection::QueueFailure<Self::Error, Self::Error, Self::Error>,
    ) -> Self::Error;
}

pub struct Queue<'a, L: Library, P: crate::gc_collection::Physical<Error = L::Error>, H> {
    prepared: crate::queue_gc::Prepared<'a>,
    physical: P,
    hooks: H,
    library: std::marker::PhantomData<L>,
}

impl<'a, L: Library, P: crate::gc_collection::Physical<Error = L::Error>, H> Queue<'a, L, P, H> {
    pub(crate) fn new(prepared: crate::queue_gc::Prepared<'a>, hooks: H, physical: P) -> Self {
        Self {
            prepared,
            hooks,
            physical,
            library: std::marker::PhantomData,
        }
    }
}

impl<
    L: Library,
    P: crate::gc_collection::Physical<Error = L::Error>,
    H,
    S: Source<Error = L::Error>,
> Call<S> for Queue<'_, L, P, H>
where
    H: for<'a> crate::wal_sink::Hooks<crate::file_adapter::NativeFile<'a, L>>,
{
    type Output = crate::gc_collection::Collected;

    fn call(self, catalog: &mut Catalog<S>) -> Result<Self::Output, L::Error> {
        match catalog.collect_queue_native::<L, H, P>(self.prepared, self.hooks, self.physical) {
            Ok(value) => Ok(value),
            Err(error) => Err(L::queue_error(error)),
        }
    }
}
