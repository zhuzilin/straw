//! Actual catalog storage context and recovery/append composition.

use crate::catalog_log::LogState;
use crate::catalog_namespace::{Adapter, Source};
use std::collections::{BTreeSet, HashMap};

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

pub struct Catalog<S: Source> {
    pub(crate) file: std::fs::File,

    pub(crate) log_identity: (u64, u64),
    pub(crate) pool_root: String,
    pub(crate) pool_run: String,
    pub(crate) log: LogState,
    pub(crate) closures: HashMap<String, BTreeSet<String>>,
    pub(crate) namespace: Adapter<S>,
    pub(crate) transaction_active: bool,
}

impl<S: Source> Catalog<S> {
    pub(crate) fn from_opened(
        opened: crate::catalog_primitives::Opened,
        log_identity: (u64, u64),
        pool_root: String,
        pool_run: String,
        source: S,
    ) -> Self {
        Self {
            file: opened.file,

            log_identity,
            pool_root,
            pool_run,
            log: LogState::new(),
            closures: HashMap::new(),
            namespace: Adapter::new(source),
            transaction_active: false,
        }
    }

    pub(crate) fn binding(&self, root: Option<&str>, run: &str) -> bool {
        match root {
            Some(root) => crate::identities::same_pool(&self.pool_root, &self.pool_run, root, run),
            None => false,
        }
    }
}

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

pub struct Handle<L: Library> {
    opened: crate::catalog_primitives::Opened,
    library: std::marker::PhantomData<L>,
}

pub struct Lock<L: Library> {
    lock: crate::catalog_primitives::Lock,
    library: std::marker::PhantomData<L>,
}

impl<L: Library> crate::catalog_transaction::Handle for Handle<L> {
    type Error = L::Error;
    fn identity(&self) -> Result<crate::catalog_transaction::Identity, L::Error> {
        match crate::catalog_transaction::Handle::identity(&self.opened) {
            Ok(identity) => Ok(identity),
            Err(error) => Err(L::io(error)),
        }
    }
}

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

/// The external caller supplies an operation preserving the catalog capability.
/// Internal operation implementations establish the same contract independently.
pub trait Call<S: Source>: Sized {
    type Output;

    fn call(self, catalog: &mut Catalog<S>) -> Result<Self::Output, S::Error>;
}

pub struct Transaction<'a, L: Library, S: Source<Error = L::Error>, C: Call<S>> {
    catalog: &'a mut Catalog<S>,
    root: Option<&'a str>,
    run: &'a str,
    call: Option<C>,
    invoking: bool,
    library: std::marker::PhantomData<L>,
}

pub struct Sync;

impl<F: crate::wal_sink::File> crate::wal_source::Barrier<F> for Sync {
    fn sync(self, file: &mut F) -> Result<(), F::Error> {
        crate::wal_sink::File::sync_all(file)
    }
}

impl<S: Source> Catalog<S> {
    pub(crate) fn refresh_native<L: Library<Error = S::Error>>(&mut self) -> Result<(), S::Error> {
        let file = crate::file_adapter::NativeFile::<L>::new(&mut self.file);
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
        self.catalog.file = fresh.opened.file;
    }
    fn refresh(&mut self, _lock: &Lock<L>) -> Result<(), L::Error> {
        self.catalog.refresh_native::<L>()
    }
    fn set_active(&mut self, active: bool) {
        self.catalog.transaction_active = active;
    }
    fn invoke(&mut self, _lock: &Lock<L>) -> Result<C::Output, L::Error> {
        self.invoking = true;
        let call = self.call.take().expect("catalog callback invoked once");
        let result = call.call(self.catalog);
        self.invoking = false;

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

impl<'a, L: Library, S: Source<Error = L::Error>, C: Call<S>> Transaction<'a, L, S, C> {
    fn new(catalog: &'a mut Catalog<S>, root: Option<&'a str>, run: &'a str, call: C) -> Self {
        Self {
            catalog,
            root,
            run,
            call: Some(call),
            invoking: false,
            library: std::marker::PhantomData,
        }
    }

    fn finish(self) {}
}

impl<S: Source> Catalog<S> {
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

// Rust invokes this guard during unwinding so a caught panic cannot leave
// partially applied catalog state available to the next caller.

impl<L: Library, S: Source<Error = L::Error>, C: Call<S>> Drop for Transaction<'_, L, S, C> {
    fn drop(&mut self) {
        if self.invoking {
            self.catalog.log.invalidate();
        }
    }
}

impl<S: Source> Catalog<S> {
    pub(crate) fn append_event<L, H>(
        &mut self,
        event: serde_json::Value,
        hooks: H,
    ) -> Result<(), crate::catalog_append::Error<S::Error, S::Error>>
    where
        L: Library<Error = S::Error>,
        H: for<'a> crate::wal_sink::Hooks<crate::file_adapter::NativeFile<'a, L>>,
    {
        let file = crate::file_adapter::NativeFile::<L>::new(&mut self.file);
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

impl<S: Source> Catalog<S> {
    pub(crate) fn bind(
        opened: crate::catalog_primitives::Opened,
        pool_root: String,
        pool_run: String,
        source: S,
    ) -> std::io::Result<Self> {
        let identity = crate::catalog_primitives::identity(&opened.file)?;
        Ok(Self::from_opened(
            opened, identity, pool_root, pool_run, source,
        ))
    }
}

impl<S: Source> Catalog<S> {
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
        let file = crate::file_adapter::NativeFile::<L>::new(&mut self.file);
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

impl<S: Source> Catalog<S> {
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
        let file = crate::file_adapter::NativeFile::<L>::new(&mut self.file);
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
