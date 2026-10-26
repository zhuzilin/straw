//! Resolve symlink targets whose final filesystem suffix may no longer exist.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Missing,
    NotLink,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    Parent,
    Current,
    Other,
}

#[derive(Debug)]
pub enum Error<E> {
    Io(E),
    Links,
    Root,
    Path,
}

pub trait Model {
    type Context;
    type Path;
    type Part;
    type IoError;
}

/// Only filesystem observations and standard Path/OsStr operations live here.
/// Successful readlink binds the target to the same stable namespace; absence
/// permits removing one nonexistent non-symlink component during recovery.
pub trait Backend: Model {
    fn canonicalize(&mut self, path: &Self::Path) -> Result<Self::Path, Self::IoError>;

    fn read_link(&mut self, path: &Self::Path) -> Result<Self::Path, Self::IoError>;

    fn classify(&self, error: &Self::IoError) -> ErrorKind;

    fn is_absolute(&self, path: &Self::Path) -> bool;

    fn join_parent(&self, path: &Self::Path, target: Self::Path) -> Option<Self::Path>;

    fn last(&self, path: &Self::Path) -> Option<Self::Part>;

    fn pop(&self, path: &mut Self::Path) -> bool;

    fn part_kind(&self, part: &Self::Part) -> PartKind;

    fn push(&self, path: &mut Self::Path, part: Self::Part);
}

pub fn resolve<S: Backend>(source: &mut S, input: S::Path) -> Result<S::Path, Error<S::IoError>> {
    let mut prefix = input;
    let mut suffix: Vec<S::Part> = Vec::new();
    let mut links = 0usize;

    loop {
        match source.canonicalize(&prefix) {
            Ok(path) => {
                return Ok(restore(source, path, suffix));
            }
            Err(error) => {
                if !matches!(source.classify(&error), ErrorKind::Missing) {
                    return Err(Error::Io(error));
                }
                match source.read_link(&prefix) {
                    Ok(target) => {
                        links += 1;
                        if links > 40 {
                            return Err(Error::Links);
                        }
                        prefix = if source.is_absolute(&target) {
                            target
                        } else {
                            match source.join_parent(&prefix, target) {
                                Some(path) => path,
                                None => return Err(Error::Root),
                            }
                        };
                    }
                    Err(link_error) => {
                        let kind = source.classify(&link_error);
                        if !matches!(kind, ErrorKind::Missing | ErrorKind::NotLink) {
                            return Err(Error::Io(link_error));
                        }

                        let part = match source.last(&prefix) {
                            Some(part) => part,
                            None => return Err(Error::Path),
                        };

                        if !source.pop(&mut prefix) {
                            return Err(Error::Io(error));
                        }

                        suffix.push(part);
                    }
                }
            }
        }
    }
}

#[inline]
fn restore<S: Backend>(source: &S, input: S::Path, parts: Vec<S::Part>) -> S::Path {
    let mut resolved = input;
    let mut suffix = parts;

    loop {
        let Some(part) = suffix.pop() else {
            return resolved;
        };

        match source.part_kind(&part) {
            PartKind::Parent => {
                source.pop(&mut resolved);
            }
            PartKind::Current => {}
            PartKind::Other => source.push(&mut resolved, part),
        }
    }
}
