//! Resolve symlink targets whose final filesystem suffix may no longer exist.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Missing,
    NotLink,
    Other,
}
#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    Parent,
    Current,
    Other,
}
#[cfg_attr(straw_verus, verus_verify)]
#[derive(Debug)]
pub enum Error<E> {
    Io(E),
    Links,
    Root,
    Path,
}

#[cfg(straw_verus)]
verus! {
    pub enum Piece { Root, Parent, Current, Name(Seq<u8>) }
    pub type Key = Seq<Piece>;
    pub open spec fn parent(key: Key) -> Option<Key> {
        if key.len() == 0 || key.last() == Piece::Root { None } else { Some(key.drop_last()) }
    }
    pub open spec fn absolute(key: Key) -> bool { key.len() > 0 && key[0] == Piece::Root }
    pub uninterp spec fn joined(base: Key, target: Key) -> Key;
    pub uninterp spec fn pushed(base: Key, part: Piece) -> Key;
    pub open spec fn append(base: Key, part: Piece) -> Key {
        match part { Piece::Parent => match parent(base) { Some(value) => value, None => base }, Piece::Current => base, _ => pushed(base, part) }
    }
    pub open spec fn rebuild(base: Key, suffix: Seq<Piece>) -> Key
        decreases suffix.len(),
    {
        if suffix.len() == 0 { base } else { append(rebuild(base, suffix.drop_first()), suffix[0]) }
    }
    pub proof fn rebuild_push(base: Key, suffix: Seq<Piece>, part: Piece)
        ensures rebuild(base, suffix.push(part)) == rebuild(append(base, part), suffix),
        decreases suffix.len(),
    {
        reveal_with_fuel(rebuild, 3);
        if suffix.len() > 0 {
            assert(suffix.push(part).drop_first() =~= suffix.drop_first().push(part));
            rebuild_push(base, suffix.drop_first(), part);
        } else {
            assert(suffix.push(part).drop_first() =~= Seq::<Piece>::empty());
        }
    }
    pub trait Model: View<V = Map<Key, Key>> {
        type Context;
        spec fn context(&self) -> Self::Context;
        type Path;
        type Part;
        type IoError;
        spec fn key(path: Self::Path) -> Key;
        spec fn piece(part: Self::Part) -> Piece;
        spec fn error(error: Self::IoError) -> ErrorKind;
    }
    pub open spec fn pieces<S: Model>(suffix: Seq<S::Part>) -> Seq<Piece> {
        Seq::new(suffix.len(), |i: int| S::piece(suffix[i]))
    }
    pub open spec fn link_meaning(namespace: Map<Key, Key>, source: Key, target: Key) -> bool {
        if absolute(target) { namespace[source] == namespace[target] }
        else { parent(source).is_some() ==> namespace[source] == namespace[joined(parent(source)->Some_0, target)] }
    }
    pub open spec fn absent_meaning(namespace: Map<Key, Key>, key: Key) -> bool {
        parent(key).is_some() ==> namespace[key] == append(namespace[parent(key)->Some_0], key.last())
    }
}
#[cfg(not(straw_verus))]
pub trait Model {
    type Context;
    type Path;
    type Part;
    type IoError;
}

/// Only filesystem observations and standard Path/OsStr operations live here.
/// Successful readlink binds the target to the same stable namespace; absence
/// permits removing one nonexistent non-symlink component during recovery.
#[cfg_attr(straw_verus, verus_verify)]
pub trait Backend: Model {
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures final(self)@ == old(self)@, final(self).context() == old(self).context(),
            result.is_ok() ==> Self::key(result->Ok_0) == old(self)@[Self::key(*path)],
    ))]
    fn canonicalize(&mut self, path: &Self::Path) -> Result<Self::Path, Self::IoError>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures final(self)@ == old(self)@, final(self).context() == old(self).context(),
            match result {
                Ok(target) => link_meaning(old(self)@, Self::key(*path), Self::key(target)),
                Err(error) => (Self::error(error) == ErrorKind::Missing || Self::error(error) == ErrorKind::NotLink)
                    ==> absent_meaning(old(self)@, Self::key(*path)),
            },
    ))]
    fn read_link(&mut self, path: &Self::Path) -> Result<Self::Path, Self::IoError>;
    #[cfg_attr(straw_verus, verus_spec(result => ensures result == Self::error(*error)))]
    fn classify(&self, error: &Self::IoError) -> ErrorKind;
    #[cfg_attr(straw_verus, verus_spec(result => ensures result == absolute(Self::key(*path))))]
    fn is_absolute(&self, path: &Self::Path) -> bool;
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures result.is_some() == parent(Self::key(*path)).is_some(),
            result.is_some() ==> Self::key(result->Some_0) == joined(parent(Self::key(*path))->Some_0, Self::key(target)),
    ))]
    fn join_parent(&self, path: &Self::Path, target: Self::Path) -> Option<Self::Path>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures match result {
            Some(part) => Self::key(*path).len() > 0 && Self::piece(part) == Self::key(*path).last(),
            None => Self::key(*path).len() == 0,
        },
    ))]
    fn last(&self, path: &Self::Path) -> Option<Self::Part>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures result == parent(Self::key(*old(path))).is_some(),
            Self::key(*final(path)) == match parent(Self::key(*old(path))) { Some(value) => value, None => Self::key(*old(path)) },
    ))]
    fn pop(&self, path: &mut Self::Path) -> bool;
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures result == match Self::piece(*part) { Piece::Parent => PartKind::Parent, Piece::Current => PartKind::Current, _ => PartKind::Other },
    ))]
    fn part_kind(&self, part: &Self::Part) -> PartKind;
    #[cfg_attr(straw_verus, verus_spec(
        ensures Self::key(*final(path)) == pushed(Self::key(*old(path)), Self::piece(part)),
    ))]
    fn push(&self, path: &mut Self::Path, part: Self::Part);
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures final(source)@ == old(source)@, final(source).context() == old(source).context(),
        result.is_ok() ==> S::key(result->Ok_0) == old(source)@[S::key(input)],
))]
pub fn resolve<S: Backend>(source: &mut S, input: S::Path) -> Result<S::Path, Error<S::IoError>> {
    let mut prefix = input;
    let mut suffix: Vec<S::Part> = Vec::new();
    let mut links = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant source@ == old(source)@, source.context() == old(source).context(), links <= 40,
            rebuild(source@[S::key(prefix)], pieces::<S>(suffix@)) == old(source)@[S::key(input)],
        decreases 40 - links, S::key(prefix).len(),
    ))]
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
                        #[cfg(straw_verus)]
                        proof! {
                            assert(S::error(link_error) == ErrorKind::Missing || S::error(link_error) == ErrorKind::NotLink);
                            assert(absent_meaning(source@, S::key(prefix)));
                        }
                        let part = match source.last(&prefix) {
                            Some(part) => part,
                            None => return Err(Error::Path),
                        };
                        #[cfg(straw_verus)]
                        let old_prefix = verus_exec_expr!(Ghost(S::key(prefix)));
                        if !source.pop(&mut prefix) {
                            return Err(Error::Io(error));
                        }
                        #[cfg(straw_verus)]
                        proof! {
                            assert(absent_meaning(source@, old_prefix@));
                            assert(S::key(prefix) == parent(old_prefix@)->Some_0);
                            assert(S::piece(part) == old_prefix@.last());
                            assert(S::key(prefix).len() < old_prefix@.len());
                            assert(source@[old_prefix@] == append(source@[S::key(prefix)], S::piece(part)));
                            rebuild_push(source@[S::key(prefix)], pieces::<S>(suffix@), S::piece(part));
                            assert(pieces::<S>(suffix@.push(part)) =~= pieces::<S>(suffix@).push(S::piece(part)));
                        }
                        suffix.push(part);
                    }
                }
            }
        }
    }
}

#[inline]
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures S::key(result) == rebuild(S::key(input), pieces::<S>(parts@)),
))]
fn restore<S: Backend>(source: &S, input: S::Path, parts: Vec<S::Part>) -> S::Path {
    let mut resolved = input;
    let mut suffix = parts;
    #[cfg_attr(straw_verus, verus_spec(
        invariant rebuild(S::key(resolved), pieces::<S>(suffix@)) == rebuild(S::key(input), pieces::<S>(parts@)),
        decreases suffix.len(),
    ))]
    loop {
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(S::key(resolved)));
        #[cfg(straw_verus)]
        let previous = verus_exec_expr!(Ghost(suffix@));
        let Some(part) = suffix.pop() else {
            return resolved;
        };
        #[cfg(straw_verus)]
        proof! {
            assert(previous@ =~= suffix@.push(part));
            assert(pieces::<S>(previous@) =~= pieces::<S>(suffix@).push(S::piece(part)));
            rebuild_push(before@, pieces::<S>(suffix@), S::piece(part));
        }
        match source.part_kind(&part) {
            PartKind::Parent => {
                source.pop(&mut resolved);
            }
            PartKind::Current => {}
            PartKind::Other => source.push(&mut resolved, part),
        }
        #[cfg(straw_verus)]
        proof! {
            rebuild_push(before@, pieces::<S>(suffix@), S::piece(part));
            assert(pieces::<S>(suffix@.push(part)) =~= pieces::<S>(suffix@).push(S::piece(part)));
        }
    }
}
