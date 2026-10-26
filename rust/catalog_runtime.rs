//! Actual catalog storage context and recovery/append composition.
#[cfg(straw_verus)]
use crate::catalog_decode::ResolverModel;
use crate::catalog_log::LogState;
use crate::catalog_namespace::{Adapter, Source};
use std::collections::{BTreeSet, HashMap};
#[cfg(straw_verus)]
use vstd::prelude::*;

pub type ReplayResult<I, N> = Result<
    crate::wal_recovery::Boundary,
    crate::wal_recovery::Failure<I, crate::catalog_target::Error<crate::catalog_frame::Error<N>>>,
>;

pub type CollectionResult<E> = Result<
    crate::gc_collection::Collected,
    crate::gc_batch::Failure<crate::gc_collection::Error<E, E, E>>,
>;
pub type QueueCollectionResult<E> =
    Result<crate::gc_collection::Collected, crate::gc_collection::QueueFailure<E, E, E>>;

#[cfg_attr(straw_verus, verus_verify)]
pub struct Catalog<S: Source> {
    pub(crate) file: std::fs::File,
    #[cfg(straw_verus)]
    pub(crate) permission: Tracked<crate::file_primitives::Permission>,
    pub(crate) log_identity: (u64, u64),
    pub(crate) pool_root: String,
    pub(crate) pool_run: String,
    pub(crate) log: LogState,
    pub(crate) closures: HashMap<String, BTreeSet<String>>,
    pub(crate) namespace: Adapter<S>,
    pub(crate) transaction_active: bool,
}
#[cfg(straw_verus)]
verus! {
    impl<S:Source> Catalog<S> {
        pub closed spec fn context(&self) -> crate::catalog_frame::Context {
            crate::catalog_frame::Context { run: self.pool_run@, namespace: self.namespace@ }
        }
        pub closed spec fn valid(&self) -> bool {
            self.permission@@.identity == crate::file_primitives::identity(self.file)
                && crate::catalog_primitives::inode(self.file) == self.log_identity
                && self.namespace.valid() && crate::catalog_log::valid(self.log@)
                && crate::catalog_target::interpreted(self.context(), self.log@.history)
                && crate::environment::string_hash_contract()
                && crate::environment::string_order_contract()
                && crate::environment::string_content_contract()
        }
        pub closed spec fn log_state(&self) -> crate::catalog_log::State { self.log@ }
        pub closed spec fn boundary(&self) -> bool {
            self.permission@@.cursor == self.log@.offset
                && self.permission@@.bytes.len() == self.log@.offset
                && self.permission@@.durable == self.log@.offset
        }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<S: Source> Catalog<S> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires opened.valid(), opened@ == log_identity,
            crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
            crate::environment::string_content_contract(), crate::namespace_identity::borrowed_strings(),
        ensures result.valid(), !result.transaction_active,
            result.context() == (crate::catalog_frame::Context { run: pool_run@, namespace: source@ }),
            result.log@.history.len() == 0,
    ))]
    pub(crate) fn from_opened(
        opened: crate::catalog_primitives::Opened,
        log_identity: (u64, u64),
        pool_root: String,
        pool_run: String,
        source: S,
    ) -> Self {
        #[cfg(straw_verus)]
        proof! { opened.facts(); }
        Self {
            file: opened.file,
            #[cfg(straw_verus)]
            permission: opened.permission,
            log_identity,
            pool_root,
            pool_run,
            log: LogState::new(),
            closures: HashMap::new(),
            namespace: Adapter::new(source),
            transaction_active: false,
        }
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures result == match root { Some(root) => self.pool_root@ == root@ && self.pool_run@ == run@, None => false },
    ))]
    pub(crate) fn binding(&self, root: Option<&str>, run: &str) -> bool {
        match root {
            Some(root) => crate::identities::same_pool(&self.pool_root, &self.pool_run, root, run),
            None => false,
        }
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::catalog_log::valid(old(log)@), crate::catalog_decode::ResolverModel::valid(&*old(namespace)), old(device).valid(),
        crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
        crate::catalog_target::interpreted(crate::catalog_frame::Context { run:run@, namespace:(*old(namespace))@ }, old(log)@.history),
    ensures crate::catalog_log::valid(final(log)@), crate::catalog_decode::ResolverModel::valid(&*final(namespace)), (*final(namespace))@ == (*old(namespace))@,
        final(device).loan() == old(device).loan(), final(device).valid(), final(device)@.identity == old(device)@.identity,
        crate::catalog_target::interpreted(crate::catalog_frame::Context { run:run@, namespace:(*old(namespace))@ }, final(log)@.history),
        result.is_ok() ==> crate::catalog_log::ready(final(log)@)
            && final(log)@.offset == final(device)@.cursor
            && final(log)@.offset == final(device)@.bytes.len()
            && final(log)@.offset == final(device)@.durable && !final(device)@.buffered,
))]
pub(crate) fn recover<S: Source, D: crate::wal_device::Device>(
    log: &mut LogState,
    namespace: &mut Adapter<S>,
    closures: &mut HashMap<String, BTreeSet<String>>,
    run: &str,
    device: &mut D,
) -> ReplayResult<D::Error, S::Error> {
    let mut decoder = crate::catalog_frame::Decoder::new(run, namespace, closures);
    let mut target = crate::catalog_target::Apply::new(log, &mut decoder);
    let result = target.recover(device);
    target.finish();
    decoder.finish();
    result
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Library: crate::file_adapter::Library {
    fn binding_error() -> Self::Error;
    fn context_error() -> Self::Error;
    fn replay_error(
        error: crate::wal_recovery::Failure<
            Self::Error,
            crate::catalog_target::Error<crate::catalog_frame::Error<Self::Error>>,
        >,
    ) -> Self::Error;
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct Handle<L: Library> {
    opened: crate::catalog_primitives::Opened,
    library: std::marker::PhantomData<L>,
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct Lock<L: Library> {
    lock: crate::catalog_primitives::Lock,
    library: std::marker::PhantomData<L>,
}
#[cfg(straw_verus)]
verus! {
    impl<L:Library> View for Handle<L> {
        type V=crate::catalog_transaction::Identity;
        closed spec fn view(&self)->Self::V { self.opened@ }
    }
    impl<L:Library> crate::catalog_transaction::HandleModel for Handle<L> {
        closed spec fn valid(&self)->bool { self.opened.valid() }
    }
    impl<L:Library> View for Lock<L> {
        type V=crate::catalog_transaction::LockView;
        closed spec fn view(&self)->Self::V { self.lock@ }
    }
    impl<L:Library> crate::catalog_transaction::LockModel for Lock<L> {}
    impl<S:Source> Catalog<S> {
        pub closed spec fn fixed(&self)->(crate::catalog_transaction::Identity,Seq<char>,crate::catalog_frame::Context) {
            (self.log_identity,self.pool_root@,self.context())
        }
        pub closed spec fn active(&self)->bool { self.transaction_active }
        pub closed spec fn ready(&self)->bool { crate::catalog_log::ready(self.log@) }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<L: Library> crate::catalog_transaction::Handle for Handle<L> {
    type Error = L::Error;
    fn identity(&self) -> Result<crate::catalog_transaction::Identity, L::Error> {
        match crate::catalog_transaction::Handle::identity(&self.opened) {
            Ok(identity) => Ok(identity),
            Err(error) => Err(L::io(error)),
        }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<L: Library> crate::catalog_transaction::Lock for Lock<L> {
    type Error = L::Error;
    fn acquire(&mut self) -> Result<(), L::Error> {
        match crate::catalog_transaction::Lock::acquire(&mut self.lock) {
            Ok(()) => Ok(()),
            Err(error) => Err(L::io(error)),
        }
    }
    fn release(&mut self) -> Result<(), L::Error> {
        match crate::catalog_transaction::Lock::release(&mut self.lock) {
            Ok(()) => Ok(()),
            Err(error) => Err(L::io(error)),
        }
    }
}

#[cfg(straw_verus)]
verus! {
    pub trait CallModel {
        spec fn admissible(&self, context:(crate::catalog_transaction::Identity,Seq<char>,crate::catalog_frame::Context))->bool;
    }
}
#[cfg(not(straw_verus))]
pub trait CallModel {}
#[cfg(not(straw_verus))]
impl<T> CallModel for T {}

/// The external caller supplies an operation preserving the catalog capability.
/// Internal operation implementations establish the same contract independently.
#[cfg_attr(straw_verus, verus_verify)]
pub trait Call<S: Source>: CallModel + Sized {
    type Output;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self.admissible(old(catalog).fixed()), old(catalog).valid(), old(catalog).active(), old(catalog).ready(), old(catalog).boundary(),
        ensures final(catalog).valid(), final(catalog).fixed() == old(catalog).fixed(),
            final(catalog).active(), final(catalog).ready() ==> final(catalog).boundary(),
    ))]
    fn call(self, catalog: &mut Catalog<S>) -> Result<Self::Output, S::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Transaction<'a, L: Library, S: Source<Error = L::Error>, C: Call<S>> {
    catalog: &'a mut Catalog<S>,
    root: Option<&'a str>,
    run: &'a str,
    call: Option<C>,
    invoking: bool,
    library: std::marker::PhantomData<L>,
    #[cfg(straw_verus)]
    refreshed: Ghost<bool>,
    #[cfg(straw_verus)]
    calls: Ghost<nat>,
    #[cfg(straw_verus)]
    origin: Ghost<(
        crate::catalog_transaction::Identity,
        Seq<char>,
        crate::catalog_frame::Context,
    )>,
}
#[cfg(straw_verus)]
verus! {
    impl<L:Library,S:Source<Error=L::Error>,C:Call<S>> View for Transaction<'_,L,S,C> {
        type V=crate::catalog_transaction::State;
        closed spec fn view(&self)->Self::V {
            crate::catalog_transaction::State {
                bound:match self.root { Some(root)=>self.catalog.pool_root@ == root@ && self.catalog.pool_run@ == self.run@, None=>false },
                active:self.catalog.transaction_active,
                ready:crate::catalog_log::ready(self.catalog.log@),
                expected:self.catalog.log_identity,
                current:crate::catalog_primitives::inode(self.catalog.file),
                refreshed:self.refreshed@,
                calls:self.calls@,
            }
        }
    }
    impl<L:Library,S:Source<Error=L::Error>,C:Call<S>> crate::catalog_transaction::Model for Transaction<'_,L,S,C> {
        type Loan = (bool, bool, bool, (crate::catalog_transaction::Identity,Seq<char>,crate::catalog_frame::Context), (crate::catalog_transaction::Identity,Seq<char>,crate::catalog_frame::Context));
        #[verifier::prophetic]
        closed spec fn loan(&self)->Self::Loan {
            (final(self.catalog).valid(), final(self.catalog).active(), final(self.catalog).ready(), final(self.catalog).fixed(), self.origin@)
        }
        closed spec fn resource_valid(&self)->bool {
            self.catalog.valid() && self.catalog.fixed() == self.origin@
                && (self.call.is_some() ==> self.call->Some_0.admissible(self.catalog.fixed())) && (self.calls@ == 0 <==> self.call.is_some())
                && (self.refreshed@ && self.catalog.ready() ==> self.catalog.boundary())
        }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Sync;
#[cfg_attr(straw_verus, verus_verify)]
impl<F: crate::wal_sink::File> crate::wal_source::Barrier<F> for Sync {
    fn sync(self, file: &mut F) -> Result<(), F::Error> {
        crate::wal_sink::File::sync_all(file)
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<S: Source> Catalog<S> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).valid(), final(self).fixed() == old(self).fixed(),
            final(self).active() == old(self).active(),
            result.is_ok() ==> final(self).ready() && final(self).boundary(),
    ))]
    pub(crate) fn refresh_native<L: Library<Error = S::Error>>(&mut self) -> Result<(), S::Error> {
        let file = crate::file_adapter::NativeFile::<L>::new(
            &mut self.file,
            #[cfg(straw_verus)]
            &mut self.permission,
        );
        let mut input = crate::wal_source::Source::from_file(file, Sync)?;
        let result = recover(
            &mut self.log,
            &mut self.namespace,
            &mut self.closures,
            &self.pool_run,
            &mut input,
        );
        input.finish();
        match result {
            Ok(_) => Ok(()),
            Err(error) => Err(L::replay_error(error)),
        }
    }
}

#[cfg(straw_verus)]
use crate::catalog_transaction::{HandleModel as _, Model as _};
#[cfg_attr(straw_verus, verus_verify)]
impl<L: Library, S: Source<Error = L::Error>, C: Call<S>> crate::catalog_transaction::Effects
    for Transaction<'_, L, S, C>
{
    type Error = L::Error;
    type Output = C::Output;
    type Handle = Handle<L>;
    type Lock = Lock<L>;
    fn check_binding(&self) -> Result<(), L::Error> {
        if self.catalog.binding(self.root, self.run) {
            Ok(())
        } else {
            Err(L::binding_error())
        }
    }
    fn active(&self) -> bool {
        self.catalog.transaction_active
    }
    fn expected(&self) -> crate::catalog_transaction::Identity {
        self.catalog.log_identity
    }
    fn clone_lock(&self) -> Result<Lock<L>, L::Error> {
        match crate::catalog_primitives::clone_lock(&self.catalog.file) {
            Ok(lock) => Ok(Lock {
                lock,
                library: std::marker::PhantomData,
            }),
            Err(error) => Err(L::io(error)),
        }
    }
    fn open_fresh(&self) -> Result<Handle<L>, L::Error> {
        match crate::catalog_primitives::open(&self.catalog.pool_root, false) {
            Ok(opened) => Ok(Handle {
                opened,
                library: std::marker::PhantomData,
            }),
            Err(error) => Err(L::io(error)),
        }
    }
    fn install(&mut self, fresh: Handle<L>) {
        #[cfg(straw_verus)]
        proof! { fresh.opened.facts(); }
        self.catalog.file = fresh.opened.file;
        #[cfg(straw_verus)]
        {
            self.catalog.permission = fresh.opened.permission;
            proof! { self.refreshed=Ghost(false); }
        }
    }
    fn refresh(&mut self, _lock: &Lock<L>) -> Result<(), L::Error> {
        let result = self.catalog.refresh_native::<L>();
        #[cfg(straw_verus)]
        proof! { self.refreshed=Ghost(result.is_ok()); }
        result
    }
    fn set_active(&mut self, active: bool) {
        self.catalog.transaction_active = active;
    }
    fn invoke(&mut self, _lock: &Lock<L>) -> Result<C::Output, L::Error> {
        self.invoking = true;
        let call = self.call.take().expect("catalog callback invoked once");
        let result = call.call(self.catalog);
        self.invoking = false;
        #[cfg(straw_verus)]
        proof! { self.calls=Ghost(self.calls@+1); }
        result
    }
    fn check_context(&self) -> Result<(), L::Error> {
        if !self.catalog.binding(self.root, self.run) {
            return Err(L::binding_error());
        }
        if !self.catalog.transaction_active || !self.catalog.log.ready() {
            return Err(L::context_error());
        }
        Ok(())
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<'a, L: Library, S: Source<Error = L::Error>, C: Call<S>> Transaction<'a, L, S, C> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(catalog).valid(), call.admissible(old(catalog).fixed()),
        ensures result.resource_valid(), crate::catalog_transaction::valid(result@),
            result@.calls == 0, !result@.refreshed,
            result@.active == old(catalog).active(), result@.ready == old(catalog).ready(),
            final(catalog).valid() == result.loan().0,
            final(catalog).active() == result.loan().1,
            final(catalog).ready() == result.loan().2,
            final(catalog).fixed() == result.loan().3, result.loan().4 == old(catalog).fixed(),
    ))]
    fn new(catalog: &'a mut Catalog<S>, root: Option<&'a str>, run: &'a str, call: C) -> Self {
        #[cfg(straw_verus)]
        let origin = verus_exec_expr!(Ghost(catalog.fixed()));
        Self {
            catalog,
            root,
            run,
            call: Some(call),
            invoking: false,
            library: std::marker::PhantomData,
            #[cfg(straw_verus)]
            refreshed: verus_exec_expr!(Ghost(false)),
            #[cfg(straw_verus)]
            calls: verus_exec_expr!(Ghost(0)),
            #[cfg(straw_verus)]
            origin,
        }
    }
    #[cfg_attr(straw_verus, verus_spec(
        requires self.resource_valid(),
        ensures self.loan().0, self.loan().1 == self@.active, self.loan().2 == self@.ready, self.loan().3 == self.loan().4,
    ))]
    fn finish(self) {}
}
#[cfg_attr(straw_verus, verus_verify)]
impl<S: Source> Catalog<S> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), call.admissible(old(self).fixed()),
        ensures final(self).valid(), final(self).fixed() == old(self).fixed(), !old(self).active() ==> !final(self).active(),
            result.is_ok() ==> final(self).ready(),
    ))]
    pub(crate) fn execute<L: Library<Error = S::Error>, C: Call<S>>(
        &mut self,
        root: Option<&str>,
        run: &str,
        call: C,
    ) -> Result<C::Output, crate::catalog_transaction::Failure<S::Error>> {
        let mut effects = Transaction::<L, S, C>::new(self, root, run, call);
        let result = crate::catalog_transaction::execute(&mut effects);
        effects.finish();
        result
    }
}

// Rust invokes this guard during unwinding. Its placement is part of the
// language-level boundary; normal Result paths above are verified directly.
#[cfg(not(straw_verus))]
impl<L: Library, S: Source<Error = L::Error>, C: Call<S>> Drop for Transaction<'_, L, S, C> {
    fn drop(&mut self) {
        if self.invoking {
            self.catalog.log.invalidate();
        }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<S: Source> Catalog<S> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), old(self).active(), old(self).ready(), old(self).boundary(),
        ensures final(self).valid(), final(self).fixed() == old(self).fixed(), final(self).active(),
            final(self).ready() ==> final(self).boundary(),
            result.is_ok() ==> final(self).ready(),
            result.is_err() ==> final(self).log@.history == old(self).log@.history
                && final(self).log_state().indexes == old(self).log_state().indexes,
    ))]
    pub(crate) fn append_event<L, H>(
        &mut self,
        event: serde_json::Value,
        hooks: H,
    ) -> Result<(), crate::catalog_append::Error<S::Error, S::Error>>
    where
        L: Library<Error = S::Error>,
        H: for<'a> crate::wal_sink::Hooks<crate::file_adapter::NativeFile<'a, L>>,
    {
        let file = crate::file_adapter::NativeFile::<L>::new(
            &mut self.file,
            #[cfg(straw_verus)]
            &mut self.permission,
        );
        let mut writer =
            crate::wal_sink::Appender::from_file(file, crate::framing::Kind::Catalog, hooks);
        let result = crate::catalog_append::append(
            &mut self.log,
            &mut self.namespace,
            &self.pool_run,
            event,
            &mut writer,
        );
        writer.finish();
        result
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<S: Source> Catalog<S> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires opened.valid(), crate::environment::string_hash_contract(), crate::environment::string_order_contract(),
            crate::environment::string_content_contract(), crate::namespace_identity::borrowed_strings(),
        ensures result.is_ok() ==> result->Ok_0.valid() && !result->Ok_0.active(),
    ))]
    pub(crate) fn bind(
        opened: crate::catalog_primitives::Opened,
        pool_root: String,
        pool_run: String,
        source: S,
    ) -> std::io::Result<Self> {
        #[cfg(straw_verus)]
        proof! { opened.facts(); }
        let identity = crate::catalog_primitives::identity(&opened.file)?;
        Ok(Self::from_opened(
            opened, identity, pool_root, pool_run, source,
        ))
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<S: Source> Catalog<S> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), old(self).active(), old(self).ready(), old(self).boundary(),
            crate::gc_collection::physical_valid(physical@), vstd::laws_cmp::obeys_cmp::<P::Parent>(),
        ensures final(self).valid(), final(self).fixed() == old(self).fixed(), final(self).active(),
            final(self).ready() ==> final(self).boundary(), result.is_ok() ==> final(self).ready(),
            result.is_ok() ==> final(self).log_state().indexes == crate::gc_collection::after_collection(
                crate::gc_collection::queue_pruned(old(self).log_state().indexes,prepared@.0,prepared@.1))
                && crate::gc_collection::live_retained(final(self).log_state().indexes,prepared@.1),
    ))]
    pub(crate) fn collect_queue_native<L, H, P>(
        &mut self,
        prepared: crate::queue_gc::Prepared<'_>,
        hooks: H,
        mut physical: P,
    ) -> QueueCollectionResult<S::Error>
    where
        L: Library<Error = S::Error>,
        P: crate::gc_collection::Physical<Error = S::Error>,
        H: for<'a> crate::wal_sink::Hooks<crate::file_adapter::NativeFile<'a, L>>,
    {
        let file = crate::file_adapter::NativeFile::<L>::new(
            &mut self.file,
            #[cfg(straw_verus)]
            &mut self.permission,
        );
        let mut writer =
            crate::wal_sink::Appender::from_file(file, crate::framing::Kind::Catalog, hooks);
        let result = crate::gc_collection::collect_queue(
            &mut self.log,
            &mut self.namespace,
            &self.pool_run,
            &mut writer,
            &mut physical,
            prepared,
        );
        writer.finish();
        result
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<S: Source> Catalog<S> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), old(self).active(), old(self).ready(), old(self).boundary(),
            crate::gc_collection::physical_valid(physical@), vstd::laws_cmp::obeys_cmp::<P::Parent>(),
        ensures final(self).valid(), final(self).fixed() == old(self).fixed(), final(self).active(),
            final(self).ready() ==> final(self).boundary(), result.is_ok() ==> final(self).ready(),
            result.is_ok() ==> final(self).log_state().indexes == crate::gc_collection::after_collection(old(self).log_state().indexes),
    ))]
    pub(crate) fn collect_native<L, H, P>(
        &mut self,
        hooks: H,
        mut physical: P,
    ) -> CollectionResult<S::Error>
    where
        L: Library<Error = S::Error>,
        P: crate::gc_collection::Physical<Error = S::Error>,
        H: for<'a> crate::wal_sink::Hooks<crate::file_adapter::NativeFile<'a, L>>,
    {
        let file = crate::file_adapter::NativeFile::<L>::new(
            &mut self.file,
            #[cfg(straw_verus)]
            &mut self.permission,
        );
        let mut writer =
            crate::wal_sink::Appender::from_file(file, crate::framing::Kind::Catalog, hooks);
        let result = crate::gc_collection::collect(
            &mut self.log,
            &mut self.namespace,
            &self.pool_run,
            &mut writer,
            &mut physical,
        );
        writer.finish();
        result
    }
}
