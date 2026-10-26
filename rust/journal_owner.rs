//! The production WAL owner, including creation, identity admission and close.
use crate::file_adapter::{Library, NativeFile};
use crate::journal_state::State;
use crate::wal_sink::Hooks;
use crate::wal_source::Barrier;
#[cfg(straw_verus)]
use crate::wal_source::FileModel;
use serde_json::Value;
use std::fs::File;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg(straw_verus)]
verus! {
    pub uninterp spec fn destination<P>(path:P) -> int;
    pub open spec fn run_header(value:Value, identity:Value) -> bool {
        match crate::json_specs::json(value) {
            crate::json_specs::Json::Array(items) => items.len()==1
                && crate::json_specs::json(items[0]) is Object
                && crate::json_specs::field(items[0],"type"@).is_some()
                && crate::json_specs::text(crate::json_specs::field(items[0],"type"@).unwrap()) == Some("Run"@)
                && crate::json_specs::field(items[0],"identity"@).is_some()
                && crate::json_specs::content(crate::json_specs::field(items[0],"identity"@).unwrap()) == crate::json_specs::content(identity)
                && forall|key:Seq<char>| key != "type"@ && key != "identity"@
                    ==> crate::json_specs::field(items[0],key).is_none(),
            _ => false,
        }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Opened {
    pub(crate) file: File,
    #[cfg(straw_verus)]
    pub(crate) permission: Tracked<crate::file_primitives::Permission>,
    #[cfg(straw_verus)]
    pub(crate) location: Ghost<int>,
    #[cfg(straw_verus)]
    pub(crate) read_only: Ghost<bool>,
    #[cfg(straw_verus)]
    pub(crate) recover: Ghost<bool>,
    #[cfg(straw_verus)]
    pub(crate) parent_synced: Ghost<bool>,
    #[cfg(straw_verus)]
    pub(crate) file_synced: Ghost<bool>,
}
#[cfg(straw_verus)]
verus! {
    impl Opened {
        pub closed spec fn valid(&self)->bool { self.permission@@.identity == crate::file_primitives::identity(self.file) }
    }
}

#[cfg(straw_verus)]
verus! { pub trait IoModel { spec fn directories_ready(&self)->Set<int>; } }
#[cfg(not(straw_verus))]
pub trait IoModel {}
#[cfg(not(straw_verus))]
impl<T> IoModel for T {}

/// Individual filesystem/fault primitives. No callback can perform replay,
/// choose the Run identity, or authorize a closed/quarantined journal.
#[cfg_attr(straw_verus, verus_verify)]
pub(crate) trait Io<P, L: Library>: IoModel {
    #[cfg_attr(straw_verus,verus_spec(result =>
        ensures result.is_ok() ==> final(self).directories_ready().contains(destination(*path)),
    ))]
    fn mkdir(&mut self, path: &P) -> Result<(), L::Error>;
    #[cfg_attr(straw_verus,verus_spec(result =>
        requires !read_only || recover, !read_only ==> self.directories_ready().contains(destination(*path)),
        ensures result.is_ok() ==> result->Ok_0.valid()
            && result->Ok_0.location@ == destination(*path)
            && result->Ok_0.read_only@ == read_only && result->Ok_0.recover@ == recover
            && !result->Ok_0.parent_synced@ && !result->Ok_0.file_synced@ && result->Ok_0.permission@@.cursor==0
            && (!recover ==> result->Ok_0.permission@@.bytes.len()==0 && result->Ok_0.permission@@.durable==0),
    ))]
    fn open(&mut self, path: &P, recover: bool, read_only: bool) -> Result<Opened, L::Error>;
    #[cfg_attr(straw_verus,verus_spec(result =>
        requires old(opened).valid(),
        ensures final(opened).valid(), final(opened).file==old(opened).file,
            final(opened).location==old(opened).location,final(opened).read_only==old(opened).read_only,
            final(opened).recover==old(opened).recover,final(opened).parent_synced==old(opened).parent_synced,
            result.is_ok() ==> final(opened).file_synced@ && final(opened).permission@@.bytes==old(opened).permission@@.bytes
                && final(opened).permission@@.cursor==old(opened).permission@@.cursor
                && final(opened).permission@@.durable==final(opened).permission@@.bytes.len(),
    ))]
    fn sync_new(&mut self, opened: &mut Opened) -> Result<(), L::Error>;
    #[cfg_attr(straw_verus,verus_spec(result =>
        requires old(opened).valid(), old(opened).location@==destination(*path),
        ensures final(opened).valid(),final(opened).file==old(opened).file,
            final(opened).location==old(opened).location,final(opened).read_only==old(opened).read_only,
            final(opened).recover==old(opened).recover,final(opened).file_synced==old(opened).file_synced,
            final(opened).permission@@==old(opened).permission@@,
            result.is_ok() ==> final(opened).parent_synced@,
    ))]
    fn sync_parent(&mut self, path: &P, opened: &mut Opened) -> Result<(), L::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Journal<P> {
    pub(crate) path: P,
    pub(crate) file: Option<File>,
    pub(crate) state: State,
    #[cfg(straw_verus)]
    pub(crate) permission: Tracked<crate::file_primitives::Permission>,
    #[cfg(straw_verus)]
    pub(crate) origin: Ghost<crate::wal_source::FileView>,
    #[cfg(straw_verus)]
    pub(crate) created: Ghost<bool>,
    #[cfg(straw_verus)]
    pub(crate) prepared: Ghost<bool>,
}
#[cfg(straw_verus)]
verus! {
    impl<P> Journal<P> {
        pub closed spec fn valid(&self)->bool {
            (self.file.is_some() ==> self.permission@@.identity==crate::file_primitives::identity(self.file->Some_0))
                && self.origin@.identity==self.permission@@.identity && (self.created@ ==> self.prepared@)
        }
        pub closed spec fn boundary(&self)->bool {
            self.permission@@.cursor==self.state.offset && self.permission@@.bytes.len()==self.state.offset
                && self.permission@@.durable==self.state.offset
        }
        pub closed spec fn is_closed(&self)->bool { self.file.is_none() }
        pub closed spec fn log_state(&self)->State { self.state }
        pub closed spec fn bytes(&self)->crate::wal_source::FileView { self.permission@@ }
        pub closed spec fn location(&self)->P { self.path }
        pub(crate) proof fn owner_facts(&self)
            ensures self.valid() == ((self.file.is_some() ==> self.permission@@.identity==crate::file_primitives::identity(self.file->Some_0))
                    && self.origin@.identity==self.permission@@.identity && (self.created@ ==> self.prepared@)),
                self.boundary() == (self.permission@@.cursor==self.state.offset && self.permission@@.bytes.len()==self.state.offset
                    && self.permission@@.durable==self.state.offset),
        {}
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum Error<E> {
    Mode,
    Closed,
    Identity,
    Io(E),
    Recover(crate::wal_recovery::Failure<E, crate::journal_recovery::Error>),
    Append(crate::journal_append::Error<E>),
}

#[cfg_attr(straw_verus,verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures run_header(result,*identity),crate::journal_recovery::events(result),
))]
#[allow(
    clippy::vec_init_then_push,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn header(identity: &Value) -> Value {
    let mut object = serde_json::Map::new();
    #[cfg(straw_verus)]
    proof! { assert forall|key:Seq<char>| crate::json_specs::field_in(crate::json_specs::entries(object),key).is_none() by {}; }
    object.insert("type".to_owned(), Value::String("Run".to_owned()));
    object.insert("identity".to_owned(), identity.clone());
    let mut items = Vec::new();
    items.push(Value::Object(object));
    Value::Array(items)
}

#[cfg_attr(straw_verus,verus_spec(result =>
    ensures result == (transactions@.len()>0 && crate::json_specs::equal(transactions@[0],*expected)),
))]
pub fn matches_header(transactions: &[Value], expected: &Value) -> bool {
    if transactions.is_empty() {
        return false;
    }
    transactions[0] == *expected
}

#[cfg_attr(straw_verus, verus_verify)]
impl<P> Journal<P> {
    #[cfg_attr(straw_verus,verus_spec(result =>
        requires opened.valid(),opened.location@==destination(path),opened.read_only@==read_only,
            !opened.recover@ ==> opened.file_synced@ && opened.parent_synced@,
        ensures result.valid(),result.path==path,result.file==Some(opened.file),
            result.permission@@==opened.permission@@,result.state.read_only==opened.read_only@,
            result.state.sequence==0,result.state.offset==0,!result.state.poisoned,
            result.state.transactions@.len()==0,result.state.incomplete_tail==0,
            result.origin@==opened.permission@@,result.created@ == !opened.recover@,
    ))]
    fn from_opened(path: P, opened: Opened, read_only: bool) -> Self {
        #[cfg(straw_verus)]
        let origin = verus_exec_expr!(Ghost(opened.permission@@));
        #[cfg(straw_verus)]
        let created = verus_exec_expr!(Ghost(!opened.recover@));
        #[cfg(straw_verus)]
        let prepared = opened.parent_synced;
        Self {
            path,
            file: Some(opened.file),
            state: State::new(read_only),
            #[cfg(straw_verus)]
            origin,
            #[cfg(straw_verus)]
            created,
            #[cfg(straw_verus)]
            prepared,
            #[cfg(straw_verus)]
            permission: opened.permission,
        }
    }

    #[cfg_attr(straw_verus,verus_spec(result =>
        ensures result == self.is_closed(),
    ))]
    pub fn closed(&self) -> bool {
        self.file.is_none()
    }

    #[cfg_attr(straw_verus,verus_spec(
        requires old(self).valid(),
        ensures final(self).valid(),final(self).is_closed(),final(self).location()==old(self).location(),
            final(self).log_state()==old(self).log_state(),final(self).bytes()==old(self).bytes(),
    ))]
    pub fn close(&mut self) {
        self.file = None;
    }

    #[cfg_attr(straw_verus,verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).valid(), final(self).path==old(self).path,
            final(self).origin==old(self).origin,final(self).created==old(self).created,
            final(self).file.is_some()==old(self).file.is_some(),
            final(self).state.read_only==old(self).state.read_only,
            final(self).state.transactions@==old(self).state.transactions@,
            final(self).state.incomplete_tail==old(self).state.incomplete_tail,
            result == Err(Error::Closed) <==> old(self).file.is_none(),
            result.is_err() ==> result->Err_0 is Closed || result->Err_0 is Append,
            result.is_err() ==> final(self).state.sequence==old(self).state.sequence
                && final(self).state.offset==old(self).state.offset,
            match result {Err(Error::Append(crate::journal_append::Error::Write(_)))=> final(self).state.poisoned,
                Err(_)=>final(self).state==old(self).state && final(self).permission@@==old(self).permission@@,_=>true},
            result.is_ok() ==> !final(self).state.poisoned && !final(self).state.read_only
                && result->Ok_0==old(self).state.sequence && final(self).state.sequence as int==old(self).state.sequence as int+1
                && final(self).boundary()
                && final(self).permission@@.bytes.take(old(self).permission@@.bytes.len() as int)==old(self).permission@@.bytes
                && crate::journal_append::accepted(old(self).state,final(self).state,*events,final(self).permission@@.bytes),
            old(self).state.poisoned || old(self).state.read_only ==> result.is_err(),
    ))]
    pub(crate) fn append_with<'a, L: Library, H: Hooks<NativeFile<'a, L>>>(
        &'a mut self,
        events: &Value,
        hooks: H,
    ) -> Result<u64, Error<L::Error>> {
        let file = match self.file.as_mut() {
            Some(file) => file,
            None => return Err(Error::Closed),
        };
        match crate::journal_io::append::<L, H>(
            file,
            &mut self.state,
            events,
            hooks,
            #[cfg(straw_verus)]
            &mut self.permission,
        ) {
            Ok(sequence) => Ok(sequence),
            Err(error) => Err(Error::Append(error)),
        }
    }

    #[cfg_attr(straw_verus,verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).valid(),final(self).path==old(self).path,
            final(self).origin==old(self).origin,final(self).created==old(self).created,
            final(self).file.is_some()==old(self).file.is_some(),
            final(self).state.read_only==old(self).state.read_only,
            result == Err(Error::Closed) <==> old(self).file.is_none(),
            result.is_err() ==> result->Err_0 is Closed || result->Err_0 is Recover,
            result.is_err() ==> final(self).state.sequence==old(self).state.sequence
                && final(self).state.offset==old(self).state.offset
                && (final(self).state==old(self).state || final(self).state.poisoned),
            result.is_ok() ==> !final(self).state.poisoned
                && final(self).state.sequence==final(self).state.transactions@.len()
                && final(self).state.sequence==result->Ok_0.sequence
                && final(self).state.offset==result->Ok_0.end
                && final(self).state.offset==final(self).permission@@.cursor
                && final(self).state.incomplete_tail==result->Ok_0.incomplete,
            result.is_ok() && !old(self).state.read_only ==> final(self).boundary(),
            result.is_ok() ==> exists|raw:Seq<Seq<u8>>| crate::journal_recovery::decoded(raw,final(self).state.transactions@)
                && crate::wal_recovery::replayed(crate::framing::Kind::Queue,old(self).permission@@.bytes,0,0,Seq::empty(),raw)
                && final(self).state.offset==crate::wal_recovery::run_end(0,raw),
    ))]
    pub(crate) fn recover_with<'a, L: Library, S: Barrier<NativeFile<'a, L>>>(
        &'a mut self,
        sync: S,
    ) -> Result<crate::wal_recovery::Boundary, Error<L::Error>> {
        let file = match self.file.as_mut() {
            Some(file) => file,
            None => return Err(Error::Closed),
        };
        match crate::journal_io::recover::<L, S>(
            file,
            &mut self.state,
            sync,
            #[cfg(straw_verus)]
            &mut self.permission,
        ) {
            Ok(boundary) => Ok(boundary),
            Err(error) => Err(Error::Recover(error)),
        }
    }
}

// A reborrow, not a second owner or a second fault handler.
#[cfg_attr(straw_verus, verus_verify)]
pub struct Borrowed<'a, I>(pub &'a mut I);
#[cfg_attr(straw_verus, verus_verify)]
impl<F: crate::wal_sink::File, I: Hooks<F>> Hooks<F> for Borrowed<'_, I> {
    fn point(&mut self, file: &mut F, point: &str) -> Result<(), F::Error> {
        self.0.point(file, point)
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait RecoverySync<F: crate::wal_source::File> {
    #[cfg_attr(straw_verus,verus_spec(result =>
        requires old(file).valid(),
        ensures final(file).loan()==old(file).loan(),final(file).valid(),final(file)@.identity==old(file)@.identity,
            result.is_ok() ==> final(file)@.bytes==old(file)@.bytes && final(file)@.durable==final(file)@.bytes.len(),
    ))]
    fn sync(&mut self, file: &mut F) -> Result<(), F::Error>;
}
#[cfg_attr(straw_verus, verus_verify)]
impl<F: crate::wal_source::File, I: RecoverySync<F>> Barrier<F> for Borrowed<'_, I> {
    fn sync(self, file: &mut F) -> Result<(), F::Error> {
        self.0.sync(file)
    }
}

#[cfg_attr(straw_verus,verus_spec(result =>
    requires crate::environment::string_order_contract(),
    ensures result == Err(Error::Mode) <==> read_only && !recover,
        result.is_ok() ==> result->Ok_0.valid() && result->Ok_0.path==path && result->Ok_0.file.is_some()
            && result->Ok_0.state.read_only==read_only && !result->Ok_0.state.poisoned
            && result->Ok_0.created@ == !recover,
        result.is_ok() && !read_only ==> result->Ok_0.boundary(),
        result.is_ok() && recover ==> result->Ok_0.state.transactions@.len()>0
            && (exists|expected:Value| run_header(expected,*identity)
                && crate::json_specs::equal(result->Ok_0.state.transactions@[0],expected))
            && (exists|raw:Seq<Seq<u8>>| crate::journal_recovery::decoded(raw,result->Ok_0.state.transactions@)
                && crate::wal_recovery::replayed(crate::framing::Kind::Queue,result->Ok_0.origin@.bytes,0,0,Seq::empty(),raw)
                && result->Ok_0.state.offset==crate::wal_recovery::run_end(0,raw)),
        result.is_ok() && !recover ==> result->Ok_0.origin@.bytes.len()==0 && result->Ok_0.state.sequence==1
            && (exists|before:State,events:Value| before.sequence==0 && before.offset==0 && run_header(events,*identity)
                && crate::journal_append::accepted(before,result->Ok_0.state,events,result->Ok_0.permission@@.bytes)),
))]
#[allow(
    clippy::collapsible_if,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub(crate) fn open<P, L: Library, I>(
    path: P,
    identity: &Value,
    recover: bool,
    read_only: bool,
    mut io: I,
) -> Result<Journal<P>, Error<L::Error>>
where
    I: Io<P, L>,
    for<'a> I: Hooks<NativeFile<'a, L>> + RecoverySync<NativeFile<'a, L>>,
{
    if read_only && !recover {
        return Err(Error::Mode);
    }
    if !read_only {
        if let Err(error) = io.mkdir(&path) {
            return Err(Error::Io(error));
        }
    }
    let mut opened = match io.open(&path, recover, read_only) {
        Ok(opened) => opened,
        Err(error) => return Err(Error::Io(error)),
    };
    if !recover {
        if let Err(error) = io.sync_new(&mut opened) {
            return Err(Error::Io(error));
        }
        if let Err(error) = io.sync_parent(&path, &mut opened) {
            return Err(Error::Io(error));
        }
    }
    let mut journal = Journal::from_opened(path, opened, read_only);
    if recover {
        journal.recover_with::<L, _>(Borrowed(&mut io))?;
        // Identity rejection, too, must inspect the recovered raw prefix.
        // An empty initial State is not evidence that the stored Run is absent.
        #[cfg(straw_verus)]
        proof! {
            assert(exists|raw:Seq<Seq<u8>>| crate::journal_recovery::decoded(raw,journal.state.transactions@)
                && crate::wal_recovery::replayed(crate::framing::Kind::Queue,journal.origin@.bytes,0,0,Seq::empty(),raw)
                && journal.state.offset==crate::wal_recovery::run_end(0,raw));
        }
        let expected = header(identity);
        if !matches_header(&journal.state.transactions, &expected) {
            return Err(Error::Identity);
        }
    } else {
        let events = header(identity);
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(journal.state));
        journal.append_with::<L, _>(&events, Borrowed(&mut io))?;
        #[cfg(straw_verus)]
        proof! {
            assert(exists|b:State,e:Value| b==before@ && e==events && b.sequence==0 && b.offset==0
                && run_header(e,*identity) && crate::journal_append::accepted(b,journal.state,e,journal.permission@@.bytes));
        }
    }
    Ok(journal)
}
