//! Bind the real deletion target snapshot to indexed I/O and receipt dispatch.
//! Raw path/syscall and directory-chain observations are explicit primitives.

use crate::{catalog_decode::Resolver, catalog_log::LogState, gc_batch, gc_io, wal_device::Writer};

pub trait PhysicalModel {
    type Parent: Ord;
    type Path;
    type Directory;
}

pub trait Physical: PhysicalModel {
    type Error;

    #[allow(
        clippy::ptr_arg,
        reason = "The physical state model uses the borrowed String as its exact key."
    )]
    fn resolve(&self, name: &String) -> Result<Self::Path, Self::Error>;

    #[allow(
        clippy::ptr_arg,
        reason = "The physical state model uses the borrowed String as its exact key."
    )]
    fn parent(&self, name: &String, path: &Self::Path) -> Self::Parent;

    fn stat(&mut self, path: &Self::Path) -> Result<Option<u64>, Self::Error>;

    #[allow(
        clippy::ptr_arg,
        reason = "The physical state model uses the borrowed String as its exact key."
    )]
    fn remove(&mut self, name: &String, path: &Self::Path) -> Result<bool, Self::Error>;

    fn open_parent(
        &mut self,
        parent: &Self::Parent,
    ) -> Result<Option<Self::Directory>, Self::Error>;

    fn sync_parent(&mut self, directory: &Self::Directory) -> Result<(), Self::Error>;
    // create_dir_all + canonical ancestor enumeration. No barrier is hidden here.

    fn prepare_parent(&mut self, parent: &Self::Parent) -> Result<Vec<Self::Parent>, Self::Error>;

    fn sync_ancestor(&mut self, parent: &Self::Parent) -> Result<(), Self::Error>;
}

pub trait Controls: Writer {
    fn restart(&mut self);

    fn point(&mut self, name: &str) -> Result<(), Self::Error>;
}

impl<F: crate::wal_sink::File, H: crate::wal_sink::Hooks<F>> Controls
    for crate::wal_sink::Appender<F, H>
{
    fn restart(&mut self) {
        crate::wal_sink::Appender::restart(self);
    }
    fn point(&mut self, name: &str) -> Result<(), Self::Error> {
        crate::wal_sink::Appender::point(self, name)
    }
}

pub enum Error<P, R, D> {
    Physical(P),
    Point(D),
    Receipt(crate::catalog_append::Error<R, D>),
    Mark(crate::catalog_append::Error<R, D>),
    Context,
    Directory,
}

pub struct Batch<'a, R: Resolver, D: Controls, P: Physical> {
    log: &'a mut LogState,
    resolver: &'a mut R,
    run: &'a str,
    io: Repair<'a, P, D>,
    targets: Vec<String>,
}

impl<R: Resolver, D: Controls, P: Physical> gc_batch::BatchModel for Batch<'_, R, D, P> {
    type Parent = P::Parent;
}

impl<R: Resolver, D: Controls, P: Physical> gc_io::Model for Batch<'_, R, D, P> {
    type Path = P::Path;
    type Directory = P::Directory;
}

impl<'a, R: Resolver, D: Controls, P: Physical> Batch<'a, R, D, P> {
    pub fn new(
        log: &'a mut LogState,
        resolver: &'a mut R,
        run: &'a str,
        device: &'a mut D,
        physical: &'a mut P,
    ) -> Self {
        let io = Repair { physical, device };
        Self {
            log,
            resolver,
            run,
            io,
            targets: Vec::new(),
        }
    }
}

pub enum RepairError<P, D> {
    Physical(P),
    Point(D),
    Incomplete,
}

struct Repair<'a, P: Physical, D: Controls> {
    physical: &'a mut P,
    device: &'a mut D,
}

impl<P: Physical, D: Controls> crate::directory_io::Barrier<P::Parent> for Repair<'_, P, D> {
    type Error = RepairError<P::Error, D::Error>;
    fn sync(&mut self, path: &P::Parent) -> Result<(), Self::Error> {
        if let Err(error) = self.device.point("before_directory_sync") {
            return Err(RepairError::Point(error));
        }
        match self.physical.sync_ancestor(path) {
            Ok(()) => Ok(()),
            Err(error) => Err(RepairError::Physical(error)),
        }
    }
    fn incomplete() -> Self::Error {
        RepairError::Incomplete
    }
}

impl<R: Resolver, D: Controls, P: Physical> gc_io::Primitives for Batch<'_, R, D, P> {
    type Error = Error<P::Error, R::Error, D::Error>;
    fn resolve(&self, index: usize) -> Result<P::Path, Self::Error> {
        match self.io.physical.resolve(&self.targets[index]) {
            Ok(path) => Ok(path),
            Err(error) => Err(Error::Physical(error)),
        }
    }
    fn parent(&self, index: usize, path: &P::Path) -> P::Parent {
        self.io.physical.parent(&self.targets[index], path)
    }
    fn point(&mut self, point: &str) -> Result<(), Self::Error> {
        match self.io.device.point(point) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.log.invalidate();
                Err(Error::Point(error))
            }
        }
    }
    fn stat(&mut self, _index: usize, path: &P::Path) -> Result<Option<u64>, Self::Error> {
        match self.io.physical.stat(path) {
            Ok(value) => Ok(value),
            Err(error) => Err(Error::Physical(error)),
        }
    }
    fn remove(&mut self, index: usize, path: &P::Path) -> Result<bool, Self::Error> {
        match self.io.physical.remove(&self.targets[index], path) {
            Ok(value) => Ok(value),
            Err(error) => Err(Error::Physical(error)),
        }
    }
    fn open_parent(&mut self, parent: &P::Parent) -> Result<Option<P::Directory>, Self::Error> {
        match self.io.physical.open_parent(parent) {
            Ok(value) => Ok(value),
            Err(error) => Err(Error::Physical(error)),
        }
    }
    fn sync(&mut self, directory: &P::Directory) -> Result<(), Self::Error> {
        let outcome = self.io.physical.sync_parent(directory);

        match outcome {
            Ok(()) => Ok(()),
            Err(error) => Err(Error::Physical(error)),
        }
    }
    fn repair(&mut self, parent: &P::Parent) -> Result<(), Self::Error> {
        let directories = match self.io.physical.prepare_parent(parent) {
            Ok(value) => value,
            Err(error) => return Err(Error::Physical(error)),
        };

        let outcome = crate::directory_io::sync_all(&directories, &mut self.io);

        if outcome.is_err() {
            self.log.invalidate();
        }
        match outcome {
            Ok(()) => Ok(()),
            Err(RepairError::Physical(error)) => Err(Error::Physical(error)),
            Err(RepairError::Point(error)) => Err(Error::Point(error)),
            Err(RepairError::Incomplete) => Err(Error::Directory),
        }
    }
}

impl<R: Resolver, D: Controls, P: Physical> gc_batch::Effects for Batch<'_, R, D, P> {
    type Error = Error<P::Error, R::Error, D::Error>;
    fn target_count(&self) -> usize {
        self.targets.len()
    }
    fn marked(&self, index: usize) -> bool {
        self.log.indexes().deleted.contains(&self.targets[index])
    }
    fn unlink(&mut self, index: usize) -> Result<gc_batch::Removal<P::Parent>, Self::Error> {
        gc_io::unlink(self, index)
    }
    fn sync_parent(&mut self, parent: &P::Parent) -> Result<(), Self::Error> {
        gc_io::sync_parent(self, parent)
    }
    fn receipt(&mut self) -> Result<(), Self::Error> {
        if self.targets.is_empty() {
            if !self.log.ready() {
                return Err(Error::Context);
            }
            return Ok(());
        }
        if let Err(error) = self.io.device.point("before_gc_receipt") {
            self.log.invalidate();
            return Err(Error::Point(error));
        }
        if !self.log.ready() {
            return Err(Error::Context);
        }

        let outcome = crate::gc_receipt::record(
            self.log,
            self.resolver,
            self.run,
            &self.targets,
            self.io.device,
        );

        match outcome {
            Ok(()) => Ok(()),
            Err(error) => Err(Error::Receipt(error)),
        }
    }
}

impl<R: Resolver, D: Controls, P: Physical> Batch<'_, R, D, P> {
    #[allow(
        clippy::type_complexity,
        reason = "Keep physical, namespace and WAL failures distinct in the checked controller contract."
    )]
    pub fn collect(
        &mut self,
    ) -> Result<gc_batch::Statistics, gc_batch::Failure<Error<P::Error, R::Error, D::Error>>> {
        if let Err(error) = gc_io::Primitives::point(self, "after_gc_tombstone") {
            return Err(gc_batch::Failure::Effect(error));
        }

        gc_batch::collect(self)
    }
}

pub struct Collected {
    pub statistics: gc_batch::Statistics,
    pub retained: usize,
}

/// Compose the refreshed root scan, durable mark, exact pending snapshot,
/// physical deletion and durable receipt in the actual native collection path.
impl<R: Resolver, D: Controls, P: Physical> Batch<'_, R, D, P> {
    #[allow(
        clippy::type_complexity,
        reason = "Keep physical, namespace and WAL failures distinct in the checked controller contract."
    )]
    pub fn run(
        &mut self,
    ) -> Result<Collected, gc_batch::Failure<Error<P::Error, R::Error, D::Error>>> {
        let crate::gc_mark::Marked { live, targets } =
            match crate::gc_mark::mark(self.log, self.resolver, self.run, self.io.device) {
                Ok(marked) => marked,
                Err(error) => return Err(gc_batch::Failure::Effect(Error::Mark(error))),
            };
        self.targets = targets;

        self.io.device.restart();
        let statistics = self.collect()?;
        Ok(Collected {
            statistics,
            retained: live.len(),
        })
    }
}

pub enum QueueFailure<P, R, D> {
    Ownership(crate::queue_gc::OwnershipError),
    Reserved,
    Prune(crate::catalog_append::Error<R, D>),
    Collect(gc_batch::Failure<Error<P, R, D>>),
}

impl<R: Resolver, D: Controls, P: Physical> Batch<'_, R, D, P> {
    #[allow(
        clippy::type_complexity,
        reason = "Keep physical, namespace and WAL failures distinct in the checked controller contract."
    )]
    pub fn run_queue(
        &mut self,
        prepared: crate::queue_gc::Prepared<'_>,
    ) -> Result<Collected, QueueFailure<P::Error, R::Error, D::Error>> {
        let (prefix, live) = prepared.parts();
        if let Err(error) = crate::queue_gc::require(
            &self.log.indexes().owners,
            &self.log.indexes().deleted,
            &live,
        ) {
            return Err(QueueFailure::Ownership(error));
        }

        let dead = crate::queue_gc::prunable(&self.log.indexes().owners, prefix, &live);

        if !crate::queue_gc::not_publication_owners(&dead) {
            return Err(QueueFailure::Reserved);
        }
        if let Err(error) =
            crate::queue_gc::release(self.log, self.resolver, self.run, &dead, self.io.device)
        {
            return Err(QueueFailure::Prune(error));
        }

        self.io.device.restart();
        let outcome = self.run();

        match outcome {
            Ok(value) => Ok(value),
            Err(error) => Err(QueueFailure::Collect(error)),
        }
    }
}

impl<R: Resolver, D: Controls, P: Physical> Batch<'_, R, D, P> {
    pub fn finish(self) {}
}

/// Collect through the real batch and return the original caller resources.
#[allow(
    clippy::type_complexity,
    reason = "Keep physical, namespace and WAL failures distinct in the checked controller contract."
)]
pub fn collect<R: Resolver, D: Controls, P: Physical>(
    log: &mut LogState,
    resolver: &mut R,
    run: &str,
    device: &mut D,
    physical: &mut P,
) -> Result<Collected, gc_batch::Failure<Error<P::Error, R::Error, D::Error>>> {
    let mut batch = Batch::new(log, resolver, run, device, physical);
    let outcome = batch.run();
    batch.finish();
    outcome
}

/// Queue root admission, prune and collection over the caller's same resources.
#[allow(
    clippy::type_complexity,
    reason = "Keep physical, namespace and WAL failures distinct in the checked controller contract."
)]
pub fn collect_queue<R: Resolver, D: Controls, P: Physical>(
    log: &mut LogState,
    resolver: &mut R,
    run: &str,
    device: &mut D,
    physical: &mut P,
    prepared: crate::queue_gc::Prepared<'_>,
) -> Result<Collected, QueueFailure<P::Error, R::Error, D::Error>> {
    let mut batch = Batch::new(log, resolver, run, device, physical);
    let outcome = batch.run_queue(prepared);
    batch.finish();
    outcome
}
