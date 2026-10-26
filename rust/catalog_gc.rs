//! Internal queue collection implements the checked transaction callback.
use crate::catalog_namespace::Source;
#[cfg(straw_verus)]
use crate::catalog_runtime::CallModel;
use crate::catalog_runtime::{Call, Catalog};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub trait Library: crate::catalog_runtime::Library {
    fn queue_error(
        error: crate::gc_collection::QueueFailure<Self::Error, Self::Error, Self::Error>,
    ) -> Self::Error;
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct Queue<'a, L: Library, P: crate::gc_collection::Physical<Error = L::Error>, H> {
    prepared: crate::queue_gc::Prepared<'a>,
    physical: P,
    hooks: H,
    library: std::marker::PhantomData<L>,
}
#[cfg(straw_verus)]
verus! {
    impl<L:Library,P:crate::gc_collection::Physical<Error=L::Error>,H> Queue<'_,L,P,H> {
        pub closed spec fn after(&self,state:crate::catalog_state::State)->crate::catalog_state::State {
            crate::gc_collection::after_collection(crate::gc_collection::queue_pruned(state,self.prepared@.0,self.prepared@.1))
        }
    }
    impl<L:Library,P:crate::gc_collection::Physical<Error=L::Error>,H> CallModel for Queue<'_,L,P,H> {
        closed spec fn admissible(&self,_context:(crate::catalog_transaction::Identity,Seq<char>,crate::catalog_frame::Context))->bool {
            crate::gc_collection::physical_valid(self.physical@) && vstd::laws_cmp::obeys_cmp::<P::Parent>()
        }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<'a, L: Library, P: crate::gc_collection::Physical<Error = L::Error>, H> Queue<'a, L, P, H> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::gc_collection::physical_valid(physical@),vstd::laws_cmp::obeys_cmp::<P::Parent>(),
        ensures forall|context:(crate::catalog_transaction::Identity,Seq<char>,crate::catalog_frame::Context)|
            result.admissible(context),
    ))]
    pub(crate) fn new(prepared: crate::queue_gc::Prepared<'a>, hooks: H, physical: P) -> Self {
        Self {
            prepared,
            hooks,
            physical,
            library: std::marker::PhantomData,
        }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
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
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures result.is_ok() ==> final(catalog).log_state().indexes == self.after(old(catalog).log_state().indexes),
    ))]
    fn call(self, catalog: &mut Catalog<S>) -> Result<Self::Output, L::Error> {
        match catalog.collect_queue_native::<L, H, P>(self.prepared, self.hooks, self.physical) {
            Ok(value) => Ok(value),
            Err(error) => Err(L::queue_error(error)),
        }
    }
}
