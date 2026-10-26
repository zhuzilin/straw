//! Actual acquisition and identity checks for an owning writer-gate permit.
//! Filesystem state is attached to the mutable backend permission; a handle's
//! logical identity is immutable and does not pretend to contain mutable data.

pub type Identity = (u64, u64);

pub trait Model {
    type File;
}

pub trait Primitives: Model {
    type Error;

    fn open(&mut self) -> Result<Self::File, Self::Error>;
    // A successful exclusive lock is the environment boundary for coherence
    // and completion/cancellation of earlier authorized remote operations.

    fn lock(&mut self, file: &Self::File, nonblocking: bool) -> Result<(), Self::Error>;

    fn identity(&mut self, file: &Self::File) -> Result<Identity, Self::Error>;

    fn read_header(&mut self, file: &mut Self::File, limit: usize) -> Result<Vec<u8>, Self::Error>;

    fn fenced(&mut self) -> Result<bool, Self::Error>;

    fn named_identity(&mut self) -> Result<Identity, Self::Error>;
}

pub enum Failure<E> {
    Io(E),
    Context,
    Identity,
    Header,
    Fenced,
}

pub struct Access<P: Primitives> {
    primitives: P,
    file: P::File,
    expected: Identity,
}

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
    if let Some(expected) = expected
        && actual != expected
    {
        return Err(Failure::Identity);
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

impl<P: Primitives> Access<P> {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Fence,
    Retired,
}

pub fn marker_name(marker: Marker) -> &'static str {
    match marker {
        Marker::Fence => "fenced",
        Marker::Retired => "retired",
    }
}

pub trait PersistenceModel {
    type File;
    type IoError;
}

pub trait Persistence: PersistenceModel {
    type Error;

    fn create(&mut self, marker: Marker) -> Result<Self::File, Self::IoError>;

    fn open(&mut self, marker: Marker) -> Result<Self::File, Self::IoError>;

    fn already_exists(&self, error: &Self::IoError) -> bool;
    fn io_error(&self, error: Self::IoError) -> Self::Error;

    fn sync_file(&mut self, file: &Self::File, marker: Marker) -> Result<(), Self::Error>;

    fn sync_directory(&mut self, marker: Marker) -> Result<(), Self::Error>;
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadKind {
    Pack,
    Sealed,
    Partial,
    Other,
}

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

pub struct Entry<P> {
    pub path: P,
    pub kind: PayloadKind,
}

pub trait CleanupModel {
    type Path;
    type IoError;
}

pub trait Cleanup: CleanupModel {
    type Error;

    fn prepare(&mut self) -> Result<(), Self::Error>;

    fn list(&mut self) -> Result<Vec<Entry<Self::Path>>, Self::Error>;

    fn sync_file(&mut self, path: &Self::Path) -> Result<(), Self::Error>;

    fn unlink(&mut self, path: &Self::Path) -> Result<(), Self::IoError>;

    fn is_missing(&self, error: &Self::IoError) -> bool;
    fn io_error(&self, error: Self::IoError) -> Self::Error;

    fn checkpoint(&mut self, before: bool) -> Result<(), Self::Error>;

    fn sync_directory(&mut self) -> Result<(), Self::Error>;
}

pub fn drain<C: Cleanup>(io: &mut C) -> Result<(), C::Error> {
    io.prepare()?;
    let entries = io.list()?;
    let mut index = 0;

    while index < entries.len() {
        match entries[index].kind {
            PayloadKind::Pack | PayloadKind::Sealed => io.sync_file(&entries[index].path)?,
            PayloadKind::Partial => {
                io.checkpoint(true)?;
                if let Err(error) = io.unlink(&entries[index].path)
                    && !io.is_missing(&error)
                {
                    return Err(io.io_error(error));
                }
                io.checkpoint(false)?;
            }
            PayloadKind::Other => {}
        }
        index += 1;
    }
    io.sync_directory()
}
