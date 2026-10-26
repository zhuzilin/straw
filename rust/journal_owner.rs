//! The production WAL owner, including creation, identity admission and close.
use crate::file_adapter::{Library, NativeFile};
use crate::journal_state::State;
use crate::wal_sink::Hooks;
use crate::wal_source::Barrier;

use serde_json::Value;
use std::fs::File;

pub struct Opened {
    pub(crate) file: File,
}

/// Individual filesystem/fault primitives. No callback can perform replay,
/// choose the Run identity, or authorize a closed/quarantined journal.
pub(crate) trait Io<P, L: Library> {
    fn mkdir(&mut self, path: &P) -> Result<(), L::Error>;

    fn open(&mut self, path: &P, recover: bool, read_only: bool) -> Result<Opened, L::Error>;

    fn sync_new(&mut self, opened: &mut Opened) -> Result<(), L::Error>;

    fn sync_parent(&mut self, path: &P, opened: &mut Opened) -> Result<(), L::Error>;
}

pub struct Journal<P> {
    pub(crate) path: P,
    pub(crate) file: Option<File>,
    pub(crate) state: State,
}

pub enum Error<E> {
    Mode,
    Closed,
    Identity,
    Io(E),
    Recover(crate::wal_recovery::Failure<E, crate::journal_recovery::Error>),
    Append(crate::journal_append::Error<E>),
}

pub fn header(identity: &Value) -> Value {
    let mut object = serde_json::Map::new();

    object.insert("type".to_owned(), Value::String("Run".to_owned()));
    object.insert("identity".to_owned(), identity.clone());
    let items = vec![Value::Object(object)];
    Value::Array(items)
}

pub fn matches_header(transactions: &[Value], expected: &Value) -> bool {
    if transactions.is_empty() {
        return false;
    }
    transactions[0] == *expected
}

impl<P> Journal<P> {
    fn from_opened(path: P, opened: Opened, read_only: bool) -> Self {
        Self {
            path,
            file: Some(opened.file),
            state: State::new(read_only),
        }
    }

    pub fn closed(&self) -> bool {
        self.file.is_none()
    }

    pub fn close(&mut self) {
        self.file = None;
    }

    pub(crate) fn append_with<'a, L: Library, H: Hooks<NativeFile<'a, L>>>(
        &'a mut self,
        events: &Value,
        hooks: H,
    ) -> Result<u64, Error<L::Error>> {
        let file = match self.file.as_mut() {
            Some(file) => file,
            None => return Err(Error::Closed),
        };
        match crate::journal_io::append::<L, H>(file, &mut self.state, events, hooks) {
            Ok(sequence) => Ok(sequence),
            Err(error) => Err(Error::Append(error)),
        }
    }

    pub(crate) fn recover_with<'a, L: Library, S: Barrier<NativeFile<'a, L>>>(
        &'a mut self,
        sync: S,
    ) -> Result<crate::wal_recovery::Boundary, Error<L::Error>> {
        let file = match self.file.as_mut() {
            Some(file) => file,
            None => return Err(Error::Closed),
        };
        match crate::journal_io::recover::<L, S>(file, &mut self.state, sync) {
            Ok(boundary) => Ok(boundary),
            Err(error) => Err(Error::Recover(error)),
        }
    }
}

// A reborrow, not a second owner or a second fault handler.

pub struct Borrowed<'a, I>(pub &'a mut I);

impl<F: crate::wal_sink::File, I: Hooks<F>> Hooks<F> for Borrowed<'_, I> {
    fn point(&mut self, file: &mut F, point: &str) -> Result<(), F::Error> {
        self.0.point(file, point)
    }
}

pub trait RecoverySync<F: crate::wal_source::File> {
    fn sync(&mut self, file: &mut F) -> Result<(), F::Error>;
}

impl<F: crate::wal_source::File, I: RecoverySync<F>> Barrier<F> for Borrowed<'_, I> {
    fn sync(self, file: &mut F) -> Result<(), F::Error> {
        self.0.sync(file)
    }
}

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
    if !read_only && let Err(error) = io.mkdir(&path) {
        return Err(Error::Io(error));
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

        let expected = header(identity);
        if !matches_header(&journal.state.transactions, &expected) {
            return Err(Error::Identity);
        }
    } else {
        let events = header(identity);

        journal.append_with::<L, _>(&events, Borrowed(&mut io))?;
    }
    Ok(journal)
}
