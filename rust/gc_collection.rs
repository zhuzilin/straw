//! Bind the real deletion target snapshot to indexed I/O and receipt dispatch.
//! Raw path/syscall and directory-chain observations are explicit primitives.
#[cfg(straw_verus)]
use crate::gc_batch::BatchModel;
use crate::{catalog_decode::Resolver, catalog_log::LogState, gc_batch, gc_io, wal_device::Writer};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg(straw_verus)]
verus! {
    pub struct PhysicalState<P, N> {
        pub parents: spec_fn(String) -> P,
        pub paths: spec_fn(String) -> N,
        pub absent: Set<String>,
        pub durable: Set<String>,
        pub repair: Option<P>,
        pub required: Set<P>,
        pub synced: Set<P>,
    }
    pub trait PhysicalModel: View<V = PhysicalState<Self::Parent, Self::Path>> {
        type Parent: Ord;
        type Path;
        type Directory: gc_io::DirectoryModel<Self::Parent>;
    }
    pub open spec fn beneath<P, N>(state: PhysicalState<P, N>, parent: P) -> Set<String> {
        state.absent.filter(|name: String| (state.parents)(name) == parent)
    }
    pub open spec fn physical_valid<P, N>(state: PhysicalState<P, N>) -> bool {
        state.durable.subset_of(state.absent) && (state.repair.is_some() && state.required.subset_of(state.synced)
            ==> beneath(state,state.repair->Some_0).subset_of(state.durable))
    }
    pub open spec fn physical_step<P, N>(before: PhysicalState<P, N>, after: PhysicalState<P, N>) -> bool {
        physical_valid(after) && before.parents == after.parents && before.paths == after.paths
            && before.absent.subset_of(after.absent) && before.durable.subset_of(after.durable)
    }
    pub open spec fn projected(names: Seq<String>, paths: Set<String>) -> ISet<nat> {
        ISet::new(|index: nat| index < names.len() && paths.contains(names[index as int]))
    }
    pub proof fn projected_all(names: Seq<String>, paths: Set<String>)
        requires projected(names,paths) == ISet::new(|i:nat| i < names.len()),
        ensures names.to_set().subset_of(paths),
    {
        assert forall|name:String| names.to_set().contains(name) implies paths.contains(name) by {
            let i=choose|i:int| 0 <= i < names.len() && names[i] == name;
            assert(projected(names,paths).contains(i as nat));
        }
    }
    pub proof fn receipt_idempotent(state:crate::catalog_state::State,names:Set<String>)
        ensures crate::gc_receipt::reclaimed(crate::gc_receipt::reclaimed(state,names),names) == crate::gc_receipt::reclaimed(state,names),
    {
        let retired=names.intersect(state.deleted);
        assert(state.pending.difference(retired).difference(retired) =~= state.pending.difference(retired));
        assert(state.packs.remove_keys(retired).remove_keys(retired) =~= state.packs.remove_keys(retired));
    }

}
#[cfg(not(straw_verus))]
pub trait PhysicalModel {
    type Parent: Ord;
    type Path;
    type Directory;
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Physical: PhysicalModel {
    type Error;
    #[cfg_attr(straw_verus, verus_spec(result => ensures result.is_ok() ==> result->Ok_0 == (self@.paths)(*name)))]
    #[allow(
        clippy::ptr_arg,
        reason = "The physical state model uses the borrowed String as its exact key."
    )]
    fn resolve(&self, name: &String) -> Result<Self::Path, Self::Error>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires *path == (self@.paths)(*name), ensures result == (self@.parents)(*name),
    ))]
    #[allow(
        clippy::ptr_arg,
        reason = "The physical state model uses the borrowed String as its exact key."
    )]
    fn parent(&self, name: &String, path: &Self::Path) -> Self::Parent;
    #[cfg_attr(straw_verus, verus_spec(result => ensures final(self)@ == old(self)@))]
    fn stat(&mut self, path: &Self::Path) -> Result<Option<u64>, Self::Error>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires physical_valid(old(self)@), *path == (old(self)@.paths)(*name),
        ensures physical_step(old(self)@,final(self)@),
            result.is_ok() ==> final(self)@.absent.contains(*name),
    ))]
    #[allow(
        clippy::ptr_arg,
        reason = "The physical state model uses the borrowed String as its exact key."
    )]
    fn remove(&mut self, name: &String, path: &Self::Path) -> Result<bool, Self::Error>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures final(self)@ == old(self)@,
            result.is_ok() && result->Ok_0.is_some() ==> result->Ok_0->Some_0@ == *parent,
    ))]
    fn open_parent(
        &mut self,
        parent: &Self::Parent,
    ) -> Result<Option<Self::Directory>, Self::Error>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires physical_valid(old(self)@),
        ensures physical_step(old(self)@,final(self)@), final(self)@.absent == old(self)@.absent,
            result.is_ok() ==> beneath(old(self)@,directory@).subset_of(final(self)@.durable),
    ))]
    fn sync_parent(&mut self, directory: &Self::Directory) -> Result<(), Self::Error>;
    // create_dir_all + canonical ancestor enumeration. No barrier is hidden here.
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires physical_valid(old(self)@),
        ensures physical_step(old(self)@,final(self)@), final(self)@.absent == old(self)@.absent,
            result.is_ok() ==> final(self)@.repair == Some(*parent)
                && final(self)@.required == result->Ok_0@.to_set()
                && final(self)@.synced == Set::<Self::Parent>::empty(),
    ))]
    fn prepare_parent(&mut self, parent: &Self::Parent) -> Result<Vec<Self::Parent>, Self::Error>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires physical_valid(old(self)@),
        ensures physical_step(old(self)@,final(self)@), final(self)@.absent == old(self)@.absent,
            final(self)@.repair == old(self)@.repair, final(self)@.required == old(self)@.required,
            old(self)@.synced.subset_of(final(self)@.synced),
            result.is_ok() ==> final(self)@.synced.contains(*parent),
    ))]
    fn sync_ancestor(&mut self, parent: &Self::Parent) -> Result<(), Self::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Controls: Writer {
    #[cfg_attr(straw_verus, verus_spec(ensures final(self).loan() == old(self).loan(), final(self)@ == old(self)@, final(self).valid() == old(self).valid()))]
    fn restart(&mut self);
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), !old(self)@.buffered,
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity, !final(self)@.buffered,
            result.is_ok() ==> final(self)@ == old(self)@,
    ))]
    fn point(&mut self, name: &str) -> Result<(), Self::Error>;
}
#[cfg_attr(straw_verus, verus_verify)]
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

#[cfg_attr(straw_verus, verus_verify)]
pub enum Error<P, R, D> {
    Physical(P),
    Point(D),
    Receipt(crate::catalog_append::Error<R, D>),
    Mark(crate::catalog_append::Error<R, D>),
    Context,
    Directory,
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Batch<'a, R: Resolver, D: Controls, P: Physical> {
    log: &'a mut LogState,
    resolver: &'a mut R,
    run: &'a str,
    io: Repair<'a, P, D>,
    targets: Vec<String>,
    #[cfg(straw_verus)]
    start: Ghost<crate::catalog_state::State>,
}

#[cfg(straw_verus)]
verus! {
    impl<R:Resolver,D:Controls,P:Physical> View for Batch<'_,R,D,P> {
        type V = gc_batch::BatchView<P::Parent>;
        closed spec fn view(&self) -> Self::V {
            gc_batch::BatchView {
                parents:self.targets@.map(|i:int,name:String| (self.io.physical@.parents)(name)),
                marked:projected(self.targets@,self.log@.indexes.deleted),
                absent:projected(self.targets@,self.io.physical@.absent),
                durable_absent:projected(self.targets@,self.io.physical@.durable),
                receipted:projected(self.targets@,crate::gc_receipt::completed(self.log@.indexes)),
            }
        }
    }
    impl<R:Resolver,D:Controls,P:Physical> gc_batch::BatchModel for Batch<'_,R,D,P> {
        type Parent = P::Parent;
        type Context = (crate::catalog_state::State,Seq<String>,Seq<char>,Map<String,String>,int,spec_fn(String)->P::Parent,spec_fn(String)->P::Path,(LogState,R,D,P),D::Loan);
        #[verifier::prophetic]
        closed spec fn context(&self) -> Self::Context {(self.start@,self.targets@,self.run@,self.resolver@,self.io.device@.identity,self.io.physical@.parents,self.io.physical@.paths,self.returned(),self.io.device.loan())}
        closed spec fn confirmed(&self) -> bool { crate::catalog_log::ready(self.log@) }
        closed spec fn context_valid(&self) -> bool {
            (crate::catalog_log::ready(self.log@) ==> crate::catalog_log::at_boundary(self.log@,self.io.device@))
                && crate::catalog_log::valid(self.log@) && self.resolver.valid() && self.io.device.valid() && !self.io.device@.buffered
                && crate::catalog_target::interpreted(crate::catalog_frame::Context{run:self.run@,namespace:self.resolver@},self.log@.history)
                && self.targets@.to_set() == self.start@.pending
                && (self.log@.indexes == self.start@ || self.log@.indexes == crate::gc_receipt::reclaimed(self.start@,self.targets@.to_set()))
                && physical_valid(self.io.physical@) && self.targets@.to_set().subset_of(self.log@.indexes.deleted)
                && (forall|name:String| self.targets@.to_set().contains(name) && !self.log@.indexes.pending.contains(name)
                    ==> self.io.physical@.durable.contains(name))
                && crate::environment::string_hash_contract() && crate::environment::string_order_contract()
                && crate::environment::string_content_contract()
        }
    }
    impl<R:Resolver,D:Controls,P:Physical> gc_io::Model for Batch<'_,R,D,P> {
        type Path = P::Path;
        type Directory = P::Directory;
        closed spec fn paths(&self) -> Map<nat,P::Path> {
            Map::new(Seq::new(self.targets@.len(),|i:int| i as nat).to_set(), |i:nat| (self.io.physical@.paths)(self.targets@[i as int]))
        }
    }
}
#[cfg(not(straw_verus))]
impl<R: Resolver, D: Controls, P: Physical> gc_batch::BatchModel for Batch<'_, R, D, P> {
    type Parent = P::Parent;
}
#[cfg(not(straw_verus))]
impl<R: Resolver, D: Controls, P: Physical> gc_io::Model for Batch<'_, R, D, P> {
    type Path = P::Path;
    type Directory = P::Directory;
}

#[cfg_attr(straw_verus, verus_verify)]
impl<'a, R: Resolver, D: Controls, P: Physical> Batch<'a, R, D, P> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires crate::catalog_log::at_boundary(log@,device@), crate::catalog_log::valid(log@), crate::catalog_log::ready(log@), resolver.valid(), device.valid(), !device@.buffered,
            crate::catalog_target::interpreted(crate::catalog_frame::Context{run:run@,namespace:resolver@},log@.history),
            physical_valid(physical@),
            crate::environment::string_hash_contract(), crate::environment::string_order_contract(), crate::environment::string_content_contract(),
        ensures result.resource_valid(), result.run_name() == run@, result.state() == old(log)@,
            result.namespace() == old(resolver)@, result.device() == old(device)@,
            result.physical() == old(physical)@, result.device_loan() == old(device).loan(),
            *final(log) == result.returned().0, *final(resolver) == result.returned().1,
            *final(device) == result.returned().2, *final(physical) == result.returned().3,
    ))]
    pub fn new(
        log: &'a mut LogState,
        resolver: &'a mut R,
        run: &'a str,
        device: &'a mut D,
        physical: &'a mut P,
    ) -> Self {
        #[cfg(straw_verus)]
        let start = verus_exec_expr!(Ghost(log@.indexes));
        #[cfg(straw_verus)]
        let initial = verus_exec_expr!(Ghost(physical@));
        #[cfg(straw_verus)]
        let writer = verus_exec_expr!(Ghost(device@));
        let io = Repair {
            physical,
            device,
            #[cfg(straw_verus)]
            attempted: verus_exec_expr!(Ghost(Seq::empty())),
            #[cfg(straw_verus)]
            initial,
            #[cfg(straw_verus)]
            writer,
        };
        Self {
            log,
            resolver,
            run,
            io,
            targets: Vec::new(),
            #[cfg(straw_verus)]
            start,
        }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum RepairError<P, D> {
    Physical(P),
    Point(D),
    Incomplete,
}
#[cfg_attr(straw_verus, verus_verify)]
struct Repair<'a, P: Physical, D: Controls> {
    physical: &'a mut P,
    device: &'a mut D,
    #[cfg(straw_verus)]
    attempted: Ghost<Seq<P::Parent>>,
    #[cfg(straw_verus)]
    initial: Ghost<PhysicalState<P::Parent, P::Path>>,
    #[cfg(straw_verus)]
    writer: Ghost<crate::wal_device::DeviceView>,
}
#[cfg(straw_verus)]
verus! {
    impl<P:Physical,D:Controls> View for Repair<'_,P,D> {
        type V = crate::directory_io::State<P::Parent>;
        closed spec fn view(&self)->Self::V { crate::directory_io::State{attempted:self.attempted@,synced:self.physical@.synced} }
    }
    impl<P:Physical,D:Controls> crate::directory_io::Model<P::Parent> for Repair<'_,P,D> {
        type Context = (PhysicalState<P::Parent,P::Path>,crate::wal_device::DeviceView,P,D,D::Loan);
        type SuccessContext = crate::wal_device::DeviceView;
        closed spec fn success_context(&self) -> Self::SuccessContext { self.device@ }
        #[verifier::prophetic]
        closed spec fn context(&self) -> Self::Context {(self.initial@,self.writer@,*final(self.physical),*final(self.device),self.device.loan())}
        closed spec fn context_valid(&self)->bool {
            physical_valid(self.physical@) && self.device.valid() && !self.device@.buffered && self.physical@.parents == self.initial@.parents && self.physical@.paths == self.initial@.paths
                && self.physical@.absent == self.initial@.absent && self.initial@.durable.subset_of(self.physical@.durable)
                && self.physical@.repair == self.initial@.repair && self.physical@.required == self.initial@.required
                && self.device@.identity == self.writer@.identity && self.device@.buffered == self.writer@.buffered
        }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<P: Physical, D: Controls> crate::directory_io::Barrier<P::Parent> for Repair<'_, P, D> {
    type Error = RepairError<P::Error, D::Error>;
    fn sync(&mut self, path: &P::Parent) -> Result<(), Self::Error> {
        #[cfg(straw_verus)]
        proof! { self.attempted=Ghost(self.attempted@.push(*path)); }
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

#[cfg_attr(straw_verus, verus_verify)]
impl<R: Resolver, D: Controls, P: Physical> gc_io::Primitives for Batch<'_, R, D, P> {
    type Error = Error<P::Error, R::Error, D::Error>;
    fn resolve(&self, index: usize) -> Result<P::Path, Self::Error> {
        #[cfg(straw_verus)]
        proof! {
            let range=Seq::new(self.targets@.len(),|i:int| i as nat);
            assert(range[index as int] == index as nat);
            assert(range.to_set().contains(index as nat));
        }
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
        #[cfg(straw_verus)]
        proof! {
            if outcome.is_ok() {
                assert forall|i:nat| gc_batch::under(old(self)@,Set::empty().insert(directory@)).contains(i)
                    implies self@.durable_absent.contains(i) by {
                    assert(old(self)@.absent.contains(i));
                    assert(beneath(old(self).io.physical@,directory@).contains(self.targets@[i as int]));
                }
            }
        }
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
        #[cfg(straw_verus)]
        let initial = verus_exec_expr!(Ghost(self.io.physical@));
        #[cfg(straw_verus)]
        proof! {
            self.io.initial=Ghost(self.io.physical@);
            self.io.writer=Ghost(self.io.device@);
            self.io.attempted=Ghost(Seq::empty());
        }
        let outcome = crate::directory_io::sync_all(&directories, &mut self.io);
        #[cfg(straw_verus)]
        proof! {
            reveal(<Repair as View>::view);
            reveal(<Repair as crate::directory_io::Model>::context_valid);
            reveal(<Repair as crate::directory_io::Model>::context);
            reveal(<Repair as crate::directory_io::Model>::success_context);
            assert(physical_step(initial@,self.io.physical@));
            if outcome.is_ok() {
                assert(self.io.physical@.required.subset_of(self.io.physical@.synced));
                assert(beneath(self.io.physical@,*parent).subset_of(self.io.physical@.durable));
                assert forall|i:nat| gc_batch::under(old(self)@,Set::empty().insert(*parent)).contains(i)
                    implies self@.durable_absent.contains(i) by {
                    assert(old(self)@.absent.contains(i));
                    assert(beneath(self.io.physical@,*parent).contains(self.targets@[i as int]));
                }
            }
        }
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

#[cfg_attr(straw_verus, verus_verify)]
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
        #[cfg(straw_verus)]
        proof! {
            projected_all(self.targets@,self.io.physical@.durable);
            receipt_idempotent(self.start@,self.targets@.to_set());
        }
        let outcome = crate::gc_receipt::record(
            self.log,
            self.resolver,
            self.run,
            &self.targets,
            self.io.device,
        );
        #[cfg(straw_verus)]
        proof! {
            assert(self.log@.indexes.deleted == old(self).log@.indexes.deleted);
            if outcome.is_ok() {
                assert(self.log@.indexes == crate::gc_receipt::reclaimed(self.start@,self.targets@.to_set()));
                assert forall|i:nat| i < self.targets@.len() implies self@.receipted.contains(i) by {
                    assert(self.targets@.to_set().contains(self.targets@[i as int]));
                }
                assert(self@.receipted =~= gc_batch::indices(self@));
            }
        }
        match outcome {
            Ok(()) => Ok(()),
            Err(error) => Err(Error::Receipt(error)),
        }
    }
}

#[cfg(straw_verus)]
verus! {
    impl<R:Resolver,D:Controls,P:Physical> Batch<'_,R,D,P> {
        #[verifier::prophetic]
        pub closed spec fn returned(&self)->(LogState,R,D,P) {(*final(self.log),*final(self.resolver),*final(self.io.device),*final(self.io.physical))}
        pub closed spec fn borrowed(&self)->(LogState,R,D,P) {(*self.log,*self.resolver,*self.io.device,*self.io.physical)}
        #[verifier::prophetic]
        pub closed spec fn device_loan(&self)->D::Loan {self.io.device.loan()}
        pub closed spec fn run_name(&self)->Seq<char> {self.run@}
        pub closed spec fn state(&self)->crate::catalog_log::State {self.log@}
        pub closed spec fn names(&self)->Seq<String> {self.targets@}
        pub closed spec fn namespace(&self)->Map<String,String> {self.resolver@}
        pub closed spec fn device(&self)->crate::wal_device::DeviceView {self.io.device@}
        pub closed spec fn physical(&self)->PhysicalState<P::Parent,P::Path> {self.io.physical@}
        pub closed spec fn resource_valid(&self)->bool {
            (crate::catalog_log::ready(self.log@) ==> crate::catalog_log::at_boundary(self.log@,self.io.device@))
                && crate::catalog_log::valid(self.log@) && self.resolver.valid() && self.io.device.valid() && !self.io.device@.buffered
                && crate::catalog_target::interpreted(crate::catalog_frame::Context{run:self.run@,namespace:self.resolver@},self.log@.history)
                && physical_valid(self.io.physical@)
                && crate::environment::string_hash_contract() && crate::environment::string_order_contract()
                && crate::environment::string_content_contract()
        }
        pub closed spec fn durable(&self)->Set<String> {self.io.physical@.durable}
        proof fn initial_snapshot(&self)
            requires self.context_valid(), self.names().to_set() == self.state().indexes.pending,
            ensures self.state().indexes == self.start@,
        {
            if self.log@.indexes != self.start@ {
                assert(self.log@.indexes == crate::gc_receipt::reclaimed(self.start@,self.targets@.to_set()));
                assert(self.targets@.to_set().subset_of(self.start@.deleted));
                assert(self.log@.indexes.pending =~= Set::<String>::empty());
                assert(self.targets@.to_set() =~= Set::<String>::empty());
                assert(self.log@.indexes.packs =~= self.start@.packs);
            }
        }
        proof fn completed_snapshot(&self)
            requires self.context_valid(), self@.receipted == gc_batch::indices(self@),
            ensures self.state().indexes == crate::gc_receipt::reclaimed(self.start@,self.names().to_set()),
        {
            if self.log@.indexes == self.start@ {
                if self.targets@.len() > 0 {
                    assert(self.targets@.to_set().contains(self.targets@[0]));
                    assert(self@.receipted.contains(0));
                }
                assert(self.targets@.len() == 0);
                assert(self.targets@.to_set() =~= Set::<String>::empty());
                assert(crate::gc_receipt::reclaimed(self.start@,self.targets@.to_set()).packs =~= self.start@.packs);
                assert(crate::gc_receipt::reclaimed(self.start@,self.targets@.to_set()).pending =~= self.start@.pending);
            }
        }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<R: Resolver, D: Controls, P: Physical> Batch<'_, R, D, P> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).context_valid(), gc_batch::valid(old(self)@), crate::catalog_log::ready(old(self).state()),
            old(self).names().to_set() == old(self).state().indexes.pending, vstd::laws_cmp::obeys_cmp::<P::Parent>(),
        ensures final(self).context_valid(), final(self).context() == old(self).context(), gc_batch::valid(final(self)@),
            result.is_ok() ==> crate::catalog_log::ready(final(self).state()) && final(self).names() == old(self).names()
                && final(self).state().indexes == crate::gc_receipt::reclaimed(old(self).state().indexes,old(self).names().to_set())
                && old(self).names().to_set().subset_of(final(self).durable())
                && final(self).state().indexes.pending == Set::<String>::empty(),
    ))]
    #[allow(
        clippy::type_complexity,
        reason = "Keep physical, namespace and WAL failures distinct in the checked controller contract."
    )]
    pub fn collect(
        &mut self,
    ) -> Result<gc_batch::Statistics, gc_batch::Failure<Error<P::Error, R::Error, D::Error>>> {
        #[cfg(straw_verus)]
        proof! { self.initial_snapshot(); }
        if let Err(error) = gc_io::Primitives::point(self, "after_gc_tombstone") {
            return Err(gc_batch::Failure::Effect(error));
        }
        let outcome = gc_batch::collect(self);
        #[cfg(straw_verus)]
        proof! {
            if outcome.is_ok() {
                self.completed_snapshot(); projected_all(self.targets@,self.io.physical@.durable);
                assert(self.log@.indexes.pending =~= Set::<String>::empty());
            }
        }
        outcome
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Collected {
    pub statistics: gc_batch::Statistics,
    pub retained: usize,
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn after_mark(state:crate::catalog_state::State)->crate::catalog_state::State {
        crate::gc_mark::marked(state,crate::gc_mark::eligible_paths(state))
    }
    pub open spec fn after_collection(state:crate::catalog_state::State)->crate::catalog_state::State {
        crate::gc_receipt::reclaimed(after_mark(state),after_mark(state).pending)
    }
}

/// Compose the refreshed root scan, durable mark, exact pending snapshot,
/// physical deletion and durable receipt in the actual native collection path.
#[cfg_attr(straw_verus, verus_verify)]
impl<R: Resolver, D: Controls, P: Physical> Batch<'_, R, D, P> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).resource_valid(), crate::catalog_log::ready(old(self).state()),
            vstd::laws_cmp::obeys_cmp::<P::Parent>(),
        ensures final(self).resource_valid(), final(self).run_name() == old(self).run_name(), final(self).returned() == old(self).returned(), final(self).device_loan() == old(self).device_loan(), final(self).namespace() == old(self).namespace(),
            final(self).device().identity == old(self).device().identity,
            final(self).state().indexes == old(self).state().indexes
                || final(self).state().indexes == after_mark(old(self).state().indexes)
                || final(self).state().indexes == after_collection(old(self).state().indexes),
            result.is_ok() ==> crate::catalog_log::ready(final(self).state()) && final(self).state().indexes == after_collection(old(self).state().indexes)
                && final(self).state().indexes.pending.is_empty()
                && after_mark(old(self).state().indexes).pending.subset_of(final(self).durable())
                && (exists|roots:Set<String>| result->Ok_0.retained == roots.len()
                    && forall|p:String| roots.contains(p) == crate::catalog_state::rooted(old(self).state().indexes,p)),
    ))]
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
        #[cfg(straw_verus)]
        proof! { self.start=Ghost(self.log@.indexes); }
        self.io.device.restart();
        let statistics = self.collect()?;
        Ok(Collected {
            statistics,
            retained: live.len(),
        })
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum QueueFailure<P, R, D> {
    Ownership(crate::queue_gc::OwnershipError),
    Reserved,
    Prune(crate::catalog_append::Error<R, D>),
    Collect(gc_batch::Failure<Error<P, R, D>>),
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn queue_pruned(state:crate::catalog_state::State,prefix:Seq<char>,live:Set<String>)->crate::catalog_state::State {
        crate::queue_gc::released(state,state.owners.dom().filter(|owner:String| crate::queue_gc::byte_prefix(owner@,prefix) && !live.contains(owner)))
    }
    pub open spec fn live_retained(state:crate::catalog_state::State,live:Set<String>)->bool {
        forall|owner:String| live.contains(owner) ==> state.owners.contains_key(owner)
            && state.owners[owner].len() > 0 && state.owners[owner].disjoint(state.deleted)
    }
    pub proof fn queue_collection_protects(state:crate::catalog_state::State,prefix:Seq<char>,live:Set<String>)
        requires live_retained(state,live),
        ensures live_retained(after_collection(queue_pruned(state,prefix,live)),live),
            forall|owner:String| state.owners.contains_key(owner) && (live.contains(owner) || !crate::queue_gc::byte_prefix(owner@,prefix))
                ==> after_collection(queue_pruned(state,prefix,live)).owners.contains_key(owner)
                    && after_collection(queue_pruned(state,prefix,live)).owners[owner] == state.owners[owner]
                    && forall|path:String| state.owners[owner].contains(path) && !state.deleted.contains(path)
                        ==> !after_collection(queue_pruned(state,prefix,live)).deleted.contains(path),
    {
        let dead=state.owners.dom().filter(|owner:String| crate::queue_gc::byte_prefix(owner@,prefix) && !live.contains(owner));
        let pruned=queue_pruned(state,prefix,live);
        crate::queue_gc::prune_preserves_protected(state,dead,prefix,live);
        assert forall|owner:String| state.owners.contains_key(owner) && (live.contains(owner) || !crate::queue_gc::byte_prefix(owner@,prefix))
            implies after_collection(pruned).owners.contains_key(owner) && after_collection(pruned).owners[owner] == state.owners[owner]
                && forall|path:String| state.owners[owner].contains(path) && !state.deleted.contains(path)
                    ==> !after_collection(pruned).deleted.contains(path) by {
            assert(pruned.owners.contains_key(owner));
            assert forall|path:String| state.owners[owner].contains(path) && !state.deleted.contains(path)
                implies !after_collection(pruned).deleted.contains(path) by {
                assert(!crate::catalog_state::eligible(pruned,path));
            }
        }
        assert forall|owner:String| live.contains(owner) implies after_collection(pruned).owners.contains_key(owner)
            && after_collection(pruned).owners[owner].len() > 0
            && after_collection(pruned).owners[owner].disjoint(after_collection(pruned).deleted) by {
            assert(state.owners.contains_key(owner));
            assert forall|path:String| after_collection(pruned).owners[owner].contains(path)
                implies !after_collection(pruned).deleted.contains(path) by {
                assert(state.owners[owner].contains(path));
                assert(!state.deleted.contains(path));
            }
        }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<R: Resolver, D: Controls, P: Physical> Batch<'_, R, D, P> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).resource_valid(), crate::catalog_log::ready(old(self).state()),
            vstd::laws_cmp::obeys_cmp::<P::Parent>(),
        ensures final(self).resource_valid(), final(self).run_name() == old(self).run_name(), final(self).returned() == old(self).returned(), final(self).device_loan() == old(self).device_loan(), final(self).namespace() == old(self).namespace(),
            final(self).device().identity == old(self).device().identity,
            match result {
                Err(QueueFailure::Ownership(_)) | Err(QueueFailure::Reserved) =>
                    final(self).state() == old(self).state() && final(self).device() == old(self).device(),
                _ => true,
            },
            result.is_ok() ==> crate::catalog_log::ready(final(self).state()) && final(self).state().indexes == after_collection(queue_pruned(old(self).state().indexes,prepared@.0,prepared@.1))
                && live_retained(final(self).state().indexes,prepared@.1),
            result.is_ok() ==> forall|owner:String| old(self).state().indexes.owners.contains_key(owner)
                && (prepared@.1.contains(owner) || !crate::queue_gc::byte_prefix(owner@,prepared@.0)
                    || crate::queue_gc::byte_prefix(owner@,"publication:"@))
                ==> final(self).state().indexes.owners.contains_key(owner)
                    && final(self).state().indexes.owners[owner] == old(self).state().indexes.owners[owner],
    ))]
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
        #[cfg(straw_verus)]
        proof! { assert(live_retained(self.log@.indexes,live@)); }
        let dead = crate::queue_gc::prunable(&self.log.indexes().owners, prefix, &live);
        #[cfg(straw_verus)]
        proof! { assert(dead@.to_set() =~= self.log@.indexes.owners.dom().filter(|owner:String| crate::queue_gc::byte_prefix(owner@,prefix@) && !live@.contains(owner))); }
        if !crate::queue_gc::not_publication_owners(&dead) {
            return Err(QueueFailure::Reserved);
        }
        if let Err(error) =
            crate::queue_gc::release(self.log, self.resolver, self.run, &dead, self.io.device)
        {
            return Err(QueueFailure::Prune(error));
        }
        #[cfg(straw_verus)]
        proof! { assert(self.log@.indexes == queue_pruned(old(self).log@.indexes,prefix@,live@)); }
        self.io.device.restart();
        let outcome = self.run();
        #[cfg(straw_verus)]
        proof! { if outcome.is_ok() {
            queue_collection_protects(old(self).log@.indexes,prefix@,live@);
            assert forall|owner:String| old(self).log@.indexes.owners.contains_key(owner)
                && crate::queue_gc::byte_prefix(owner@,"publication:"@)
                implies !dead@.to_set().contains(owner) by {
                if dead@.to_set().contains(owner) {
                    let i=choose|i:int| 0 <= i < dead@.len() && dead@[i] == owner;
                    assert(!crate::queue_gc::byte_prefix(dead@[i]@,"publication:"@));
                }
            }
        } }
        match outcome {
            Ok(value) => Ok(value),
            Err(error) => Err(QueueFailure::Collect(error)),
        }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<R: Resolver, D: Controls, P: Physical> Batch<'_, R, D, P> {
    #[cfg_attr(straw_verus, verus_spec(
        requires self.resource_valid(),
        ensures self.returned() == self.borrowed(),
            self.returned().0@ == self.state(), self.returned().1@ == self.namespace(),
            self.returned().1.valid(), self.returned().2@ == self.device(), self.returned().2.valid(),
            self.returned().2.loan() == self.device_loan(), self.returned().3@ == self.physical(),
    ))]
    pub fn finish(self) {}
}

/// Collect through the real batch and return the original caller resources.
#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::catalog_log::at_boundary(old(log)@,old(device)@), crate::catalog_log::valid(old(log)@), crate::catalog_log::ready(old(log)@),
        old(resolver).valid(), old(device).valid(), !old(device)@.buffered,
        crate::catalog_target::interpreted(crate::catalog_frame::Context{run:run@,namespace:old(resolver)@},old(log)@.history),
        physical_valid(old(physical)@), vstd::laws_cmp::obeys_cmp::<P::Parent>(),
        crate::environment::string_hash_contract(), crate::environment::string_order_contract(), crate::environment::string_content_contract(),
    ensures crate::catalog_log::ready(final(log)@) ==> crate::catalog_log::at_boundary(final(log)@,final(device)@),
        result.is_ok() ==> crate::catalog_log::ready(final(log)@),
        crate::catalog_log::valid(final(log)@), final(resolver).valid(), final(resolver)@ == old(resolver)@,
        final(device).valid(), !final(device)@.buffered, final(device).loan() == old(device).loan(),
        final(device)@.identity == old(device)@.identity, physical_valid(final(physical)@),
        crate::catalog_target::interpreted(crate::catalog_frame::Context{run:run@,namespace:final(resolver)@},final(log)@.history),
        final(log)@.indexes == old(log)@.indexes || final(log)@.indexes == after_mark(old(log)@.indexes)
            || final(log)@.indexes == after_collection(old(log)@.indexes),
        result.is_ok() ==> final(log)@.indexes == after_collection(old(log)@.indexes)
            && final(log)@.indexes.pending.is_empty()
            && after_mark(old(log)@.indexes).pending.subset_of(final(physical)@.durable),
))]
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
#[cfg_attr(straw_verus, verus_spec(result =>
    requires crate::catalog_log::at_boundary(old(log)@,old(device)@), crate::catalog_log::valid(old(log)@), crate::catalog_log::ready(old(log)@),
        old(resolver).valid(), old(device).valid(), !old(device)@.buffered,
        crate::catalog_target::interpreted(crate::catalog_frame::Context{run:run@,namespace:old(resolver)@},old(log)@.history),
        physical_valid(old(physical)@), vstd::laws_cmp::obeys_cmp::<P::Parent>(),
        crate::environment::string_hash_contract(), crate::environment::string_order_contract(), crate::environment::string_content_contract(),
    ensures crate::catalog_log::ready(final(log)@) ==> crate::catalog_log::at_boundary(final(log)@,final(device)@),
        result.is_ok() ==> crate::catalog_log::ready(final(log)@),
        crate::catalog_log::valid(final(log)@), final(resolver).valid(), final(resolver)@ == old(resolver)@,
        final(device).valid(), !final(device)@.buffered, final(device).loan() == old(device).loan(),
        final(device)@.identity == old(device)@.identity, physical_valid(final(physical)@),
        crate::catalog_target::interpreted(crate::catalog_frame::Context{run:run@,namespace:final(resolver)@},final(log)@.history),
        match result {
            Err(QueueFailure::Ownership(_)) | Err(QueueFailure::Reserved) => final(log)@ == old(log)@ && final(device)@ == old(device)@,
            _ => true,
        },
        result.is_ok() ==> final(log)@.indexes == after_collection(queue_pruned(old(log)@.indexes,prepared@.0,prepared@.1))
            && live_retained(final(log)@.indexes,prepared@.1),
        result.is_ok() ==> forall|owner:String| old(log)@.indexes.owners.contains_key(owner)
            && (prepared@.1.contains(owner) || !crate::queue_gc::byte_prefix(owner@,prepared@.0)
                || crate::queue_gc::byte_prefix(owner@,"publication:"@))
            ==> final(log)@.indexes.owners.contains_key(owner)
                && final(log)@.indexes.owners[owner] == old(log)@.indexes.owners[owner],
))]
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
