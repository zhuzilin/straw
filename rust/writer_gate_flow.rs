//! Actual acquisition and identity checks for an owning writer-gate permit.
//! Filesystem state is attached to the mutable backend permission; a handle's
//! logical identity is immutable and does not pretend to contain mutable data.
#[cfg(straw_verus)]
use vstd::prelude::*;
pub type Identity = (u64, u64);
#[cfg(straw_verus)]
verus! {
    pub struct State { pub gate:Identity,pub named:Identity,pub held:bool,pub header:Seq<u8>,pub fenced:bool,pub drained:bool }
    pub trait Model: View<V=State> {
        type File;
        spec fn handle_identity(file:Self::File)->Identity;
    }
}
#[cfg(not(straw_verus))]
pub trait Model {
    type File;
}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Primitives: Model {
    type Error;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires !old(self)@.held,
        ensures final(self)@==old(self)@,result.is_ok() ==> Self::handle_identity(result->Ok_0)==old(self)@.gate,
    ))]
    fn open(&mut self) -> Result<Self::File, Self::Error>;
    // A successful exclusive lock is the environment boundary for coherence
    // and completion/cancellation of earlier authorized remote operations.
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires !old(self)@.held,Self::handle_identity(*file)==old(self)@.gate,
        ensures final(self)@.gate==old(self)@.gate,
            result.is_ok() ==> final(self)@.held && final(self)@.drained,
    ))]
    fn lock(&mut self, file: &Self::File, nonblocking: bool) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self)@.held,Self::handle_identity(*file)==old(self)@.gate,
        ensures final(self)@==old(self)@,result.is_ok() ==> result->Ok_0==Self::handle_identity(*file),
    ))]
    fn identity(&mut self, file: &Self::File) -> Result<Identity, Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self)@.held,Self::handle_identity(*old(file))==old(self)@.gate,
        ensures final(self)@==old(self)@,Self::handle_identity(*final(file))==Self::handle_identity(*old(file)),
            result.is_ok() ==> result->Ok_0@==old(self)@.header.take(if limit<old(self)@.header.len() {limit as int} else {old(self)@.header.len() as int}),
    ))]
    fn read_header(&mut self, file: &mut Self::File, limit: usize) -> Result<Vec<u8>, Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self)@.held,
        ensures final(self)@==old(self)@,result.is_ok() ==> result->Ok_0==old(self)@.fenced,
    ))]
    fn fenced(&mut self) -> Result<bool, Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self)@.held,
        ensures final(self)@==(State{named:final(self)@.named,..old(self)@}),
            result.is_ok() ==> result->Ok_0==final(self)@.named,
    ))]
    fn named_identity(&mut self) -> Result<Identity, Self::Error>;
}
#[cfg_attr(straw_verus, verus_verify)]
pub enum Failure<E> {
    Io(E),
    Context,
    Identity,
    Header,
    Fenced,
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct Access<P: Primitives> {
    primitives: P,
    file: P::File,
    expected: Identity,
}
#[cfg(straw_verus)]
verus! {
    impl<P:Primitives> Access<P> {
        pub closed spec fn held(&self)->bool {
            self.primitives@.held && self.primitives@.drained
                && P::handle_identity(self.file)==self.expected && self.expected==self.primitives@.gate
        }
        pub closed spec fn header(&self)->Seq<u8> {self.primitives@.header}
        pub closed spec fn fenced(&self)->bool {self.primitives@.fenced}
        pub closed spec fn expected_id(&self)->Identity {self.expected}
        pub closed spec fn snapshot(&self)->State {self.primitives@}
        pub closed spec fn file_id(&self)->Identity {P::handle_identity(self.file)}
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>
    requires !primitives@.held,
    ensures result.is_ok() ==> result->Ok_0.held() && result->Ok_0.header()==header@
        && (allow_fenced || !result->Ok_0.fenced()) && context_matches
        && (expected.is_some() ==> result->Ok_0.expected_id()==expected->Some_0),
))]
#[allow(
    clippy::collapsible_if,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn acquire<P: Primitives>(
    mut primitives: P,
    expected: Option<Identity>,
    header: &[u8],
    context_matches: bool,
    allow_fenced: bool,
    nonblocking: bool,
) -> Result<Access<P>, Failure<P::Error>> {
    if !context_matches {
        return Err(Failure::Context);
    }
    let mut file = match primitives.open() {
        Ok(file) => file,
        Err(error) => return Err(Failure::Io(error)),
    };
    if let Err(error) = primitives.lock(&file, nonblocking) {
        return Err(Failure::Io(error));
    }
    let actual = match primitives.identity(&file) {
        Ok(identity) => identity,
        Err(error) => return Err(Failure::Io(error)),
    };
    if let Some(expected) = expected {
        if actual != expected {
            return Err(Failure::Identity);
        }
    }
    let limit = match header.len().checked_add(1) {
        Some(limit) => limit,
        None => return Err(Failure::Header),
    };
    let raw = match primitives.read_header(&mut file, limit) {
        Ok(raw) => raw,
        Err(error) => return Err(Failure::Io(error)),
    };
    if raw.as_slice() != header {
        return Err(Failure::Header);
    }
    #[cfg(straw_verus)]
    proof! {
        assert(primitives@.header.len()<limit);
        assert(primitives@.header.take(primitives@.header.len() as int) =~= primitives@.header);
    }
    if !allow_fenced {
        match primitives.fenced() {
            Ok(true) => return Err(Failure::Fenced),
            Ok(false) => {}
            Err(error) => return Err(Failure::Io(error)),
        }
    }
    Ok(Access {
        primitives,
        file,
        expected: actual,
    })
}
#[cfg_attr(straw_verus, verus_verify)]
impl<P: Primitives> Access<P> {
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self).held(),
        ensures final(self).held(), final(self).expected_id()==old(self).expected_id(),
            final(self).snapshot()==(State{named:final(self).snapshot().named,..old(self).snapshot()}),
            final(self).file_id()==old(self).file_id(),
            result.is_ok() ==> final(self).snapshot().named==final(self).expected_id(),
    ))]
    pub fn bound(&mut self) -> Result<(), Failure<P::Error>> {
        let held = match self.primitives.identity(&self.file) {
            Ok(identity) => identity,
            Err(error) => return Err(Failure::Io(error)),
        };
        let named = match self.primitives.named_identity() {
            Ok(identity) => identity,
            Err(error) => return Err(Failure::Io(error)),
        };
        if held != self.expected || named != self.expected {
            return Err(Failure::Identity);
        }
        Ok(())
    }
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Fence,
    Retired,
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures result@==(match marker {Marker::Fence=>"fenced"@,Marker::Retired=>"retired"@})))]
pub fn marker_name(marker: Marker) -> &'static str {
    match marker {
        Marker::Fence => "fenced",
        Marker::Retired => "retired",
    }
}
#[cfg(straw_verus)]
verus! {
    pub struct PersistenceState {pub gate:Identity,pub held:bool,pub file_synced:Set<Marker>,pub name_synced:Set<Marker>}
    pub trait PersistenceModel:View<V=PersistenceState> {
        type File;
        type IoError;
        spec fn named(file:Self::File,gate:Identity,marker:Marker)->bool;
        spec fn exists(error:Self::IoError)->bool;
    }
    pub open spec fn persistence_preserved(before:PersistenceState,after:PersistenceState)->bool {
        before.gate==after.gate && before.held==after.held
            && before.file_synced.subset_of(after.file_synced) && before.name_synced.subset_of(after.name_synced)
    }
}
#[cfg(not(straw_verus))]
pub trait PersistenceModel {
    type File;
    type IoError;
}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Persistence: PersistenceModel {
    type Error;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self)@.held,
        ensures persistence_preserved(old(self)@,final(self)@),result.is_ok() ==> Self::named(result->Ok_0,old(self)@.gate,marker),
    ))]
    fn create(&mut self, marker: Marker) -> Result<Self::File, Self::IoError>;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self)@.held,
        ensures persistence_preserved(old(self)@,final(self)@),result.is_ok() ==> Self::named(result->Ok_0,old(self)@.gate,marker),
    ))]
    fn open(&mut self, marker: Marker) -> Result<Self::File, Self::IoError>;
    #[cfg_attr(straw_verus,verus_spec(result=>ensures result==Self::exists(*error)))]
    fn already_exists(&self, error: &Self::IoError) -> bool;
    fn io_error(&self, error: Self::IoError) -> Self::Error;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self)@.held,Self::named(*file,old(self)@.gate,marker),
        ensures persistence_preserved(old(self)@,final(self)@),result.is_ok() ==> final(self)@.file_synced.contains(marker),
    ))]
    fn sync_file(&mut self, file: &Self::File, marker: Marker) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self)@.held,old(self)@.file_synced.contains(marker),
        ensures persistence_preserved(old(self)@,final(self)@),result.is_ok() ==> final(self)@.name_synced.contains(marker),
    ))]
    fn sync_directory(&mut self, marker: Marker) -> Result<(), Self::Error>;
}
#[cfg_attr(straw_verus,verus_spec(result=>
    requires old(io)@.held,
    ensures persistence_preserved(old(io)@,final(io)@),
        result.is_ok() ==> final(io)@.file_synced.contains(marker) && final(io)@.name_synced.contains(marker),
))]
pub fn persist_marker<P: Persistence>(io: &mut P, marker: Marker) -> Result<(), P::Error> {
    let file = match io.create(marker) {
        Ok(file) => file,
        Err(error) => {
            if io.already_exists(&error) {
                match io.open(marker) {
                    Ok(file) => file,
                    Err(error) => return Err(io.io_error(error)),
                }
            } else {
                return Err(io.io_error(error));
            }
        }
    };
    io.sync_file(&file, marker)?;
    io.sync_directory(marker)
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadKind {
    Pack,
    Sealed,
    Partial,
    Other,
}
#[cfg_attr(straw_verus,verus_spec(result=>
    ensures result==(if extension@=="pack"@ {PayloadKind::Pack} else if extension@=="sealed"@ {PayloadKind::Sealed} else if extension@=="partial"@ {PayloadKind::Partial} else {PayloadKind::Other}),
))]
pub fn payload_kind(extension: &str) -> PayloadKind {
    if extension == "pack" {
        PayloadKind::Pack
    } else if extension == "sealed" {
        PayloadKind::Sealed
    } else if extension == "partial" {
        PayloadKind::Partial
    } else {
        PayloadKind::Other
    }
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct Entry<P> {
    pub path: P,
    pub kind: PayloadKind,
}
#[cfg(straw_verus)]
verus! {
    pub struct CleanupState<P> {pub gate:Identity,pub held:bool,pub prepared:bool,pub entries:Seq<Entry<P>>,pub synced:Set<P>,pub removed:Set<P>,pub directory_synced:bool}
    pub trait CleanupModel:View<V=CleanupState<Self::Path>> {
        type Path;
        type IoError;
        spec fn missing(error:Self::IoError)->bool;
        spec fn kind(path:Self::Path)->PayloadKind;
        spec fn local(gate:Identity,path:Self::Path)->bool;
    }
    pub open spec fn cleanup_preserved<P>(before:CleanupState<P>,after:CleanupState<P>)->bool {
        before.gate==after.gate && before.held==after.held && before.synced.subset_of(after.synced) && before.removed.subset_of(after.removed)
    }
    pub open spec fn entry_drained<P>(state:CleanupState<P>,entry:Entry<P>)->bool {
        match entry.kind {PayloadKind::Pack|PayloadKind::Sealed=>state.synced.contains(entry.path),PayloadKind::Partial=>state.removed.contains(entry.path),PayloadKind::Other=>true}
    }
}
#[cfg(not(straw_verus))]
pub trait CleanupModel {
    type Path;
    type IoError;
}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Cleanup: CleanupModel {
    type Error;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self)@.held,
        ensures cleanup_preserved(old(self)@,final(self)@),final(self)@.removed==old(self)@.removed,result.is_ok() ==> final(self)@.prepared,
    ))]
    fn prepare(&mut self) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self)@.held,old(self)@.prepared,
        ensures cleanup_preserved(old(self)@,final(self)@),final(self)@.prepared,
            final(self)@.removed==old(self)@.removed,
            result.is_ok() ==> result->Ok_0@==final(self)@.entries && forall|i:int| 0 <= i < result->Ok_0.len()
                ==> result->Ok_0@[i].kind==Self::kind(result->Ok_0@[i].path) && Self::local(final(self)@.gate,result->Ok_0@[i].path),
    ))]
    fn list(&mut self) -> Result<Vec<Entry<Self::Path>>, Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self)@.held,exists|i:int| 0 <= i < old(self)@.entries.len() && old(self)@.entries[i].path==*path
            && (old(self)@.entries[i].kind==PayloadKind::Pack || old(self)@.entries[i].kind==PayloadKind::Sealed),
        ensures cleanup_preserved(old(self)@,final(self)@),final(self)@.entries==old(self)@.entries,
            final(self)@.removed==old(self)@.removed,result.is_ok() ==> final(self)@.synced.contains(*path),
    ))]
    fn sync_file(&mut self, path: &Self::Path) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self)@.held,exists|i:int| 0 <= i < old(self)@.entries.len() && old(self)@.entries[i].path==*path && old(self)@.entries[i].kind==PayloadKind::Partial,
        ensures cleanup_preserved(old(self)@,final(self)@),final(self)@.entries==old(self)@.entries,
            final(self)@.removed.subset_of(old(self)@.removed.insert(*path)),
            (result.is_ok() || (result.is_err() && Self::missing(result->Err_0))) ==> final(self)@.removed.contains(*path),
    ))]
    fn unlink(&mut self, path: &Self::Path) -> Result<(), Self::IoError>;
    #[cfg_attr(straw_verus,verus_spec(result=>ensures result==Self::missing(*error)))]
    fn is_missing(&self, error: &Self::IoError) -> bool;
    fn io_error(&self, error: Self::IoError) -> Self::Error;
    #[cfg_attr(straw_verus,verus_spec(ensures final(self)@==old(self)@))]
    fn checkpoint(&mut self, before: bool) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus,verus_spec(result=>
        requires old(self)@.held,forall|i:int| 0 <= i < old(self)@.entries.len() ==> entry_drained(old(self)@,old(self)@.entries[i]),
        ensures cleanup_preserved(old(self)@,final(self)@),final(self)@.entries==old(self)@.entries,
            final(self)@.removed==old(self)@.removed,result.is_ok() ==> final(self)@.directory_synced,
    ))]
    fn sync_directory(&mut self) -> Result<(), Self::Error>;
}
#[cfg_attr(straw_verus,verus_spec(result=>
    requires old(io)@.held,
    ensures cleanup_preserved(old(io)@,final(io)@),
        result.is_ok() ==> final(io)@.directory_synced && forall|i:int| 0 <= i < final(io)@.entries.len() ==> entry_drained(final(io)@,final(io)@.entries[i]),
        forall|path:C::Path| final(io)@.removed.contains(path) && !old(io)@.removed.contains(path)
            ==> C::kind(path)==PayloadKind::Partial && C::local(old(io)@.gate,path),
))]
#[allow(
    clippy::collapsible_if,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn drain<C: Cleanup>(io: &mut C) -> Result<(), C::Error> {
    io.prepare()?;
    let entries = io.list()?;
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index<=entries.len(),io@.held,io@.entries==entries@,cleanup_preserved(old(io)@,io@),
            forall|i:int| 0 <= i < index ==> entry_drained(io@,entries@[i]),
            forall|i:int| 0 <= i < entries.len() ==> entries@[i].kind==C::kind(entries@[i].path) && C::local(io@.gate,entries@[i].path),
            forall|path:C::Path| io@.removed.contains(path) && !old(io)@.removed.contains(path)
                ==> C::kind(path)==PayloadKind::Partial && C::local(old(io)@.gate,path),
        decreases entries.len()-index,
    ))]
    while index < entries.len() {
        match entries[index].kind {
            PayloadKind::Pack | PayloadKind::Sealed => io.sync_file(&entries[index].path)?,
            PayloadKind::Partial => {
                io.checkpoint(true)?;
                if let Err(error) = io.unlink(&entries[index].path) {
                    if !io.is_missing(&error) {
                        return Err(io.io_error(error));
                    }
                }
                io.checkpoint(false)?;
            }
            PayloadKind::Other => {}
        }
        index += 1;
    }
    io.sync_directory()
}
