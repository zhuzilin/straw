//! Namespace identity cache and the actual ordered component resolver.
use std::collections::HashMap;

#[derive(Debug)]
pub enum Error<E> {
    Invalid,
    Pool,
    Changed,
    Read(E),
}

fn advance(mode: u8, byte: u8) -> Option<u8> {
    if byte == b'\\' {
        None
    } else if byte == b'/' {
        if mode == 3 { Some(0) } else { None }
    } else if byte == b'.' {
        Some(if mode == 0 {
            1
        } else if mode == 1 {
            2
        } else {
            3
        })
    } else {
        Some(3)
    }
}

pub fn valid(bytes: &[u8], absolute: bool) -> bool {
    if absolute {
        return false;
    }
    let mut mode = 0u8;
    let mut index = 0usize;

    while index < bytes.len() {
        let Some(next) = advance(mode, bytes[index]) else {
            return false;
        };
        mode = next;
        index += 1;
    }

    mode == 3
}

pub trait BackendModel {
    type Position;
}

/// The native adapter is a borrowed str::split('/'), with no copied components.
pub trait Parts {
    fn next(&mut self) -> Option<&str>;
}

/// Per-component filesystem/path effects, independent of the identity cache.
/// The implementation must bind each Position to the original prefix even
/// when several original names refer to the same canonical destination.
pub trait Backend: BackendModel {
    type Error;

    #[allow(clippy::ptr_arg)]
    fn root(&self, prefix: &str) -> Self::Position;

    fn cached_position(&self, prefix: &str, identity: &str) -> Self::Position;

    #[allow(clippy::ptr_arg)]
    fn step(
        &mut self,
        position: Self::Position,
        component: &str,
        prefix: &str,
    ) -> Result<(Self::Position, String), Self::Error>;
}

pub fn identity<S: Backend, P: Parts>(
    source: &mut S,
    cache: &mut HashMap<String, String>,
    name: &str,
    absolute: bool,
    parts: &mut P,
) -> Result<String, Error<S::Error>> {
    if !valid(name.as_bytes(), absolute) {
        return Err(Error::Invalid);
    }
    if let Some(identity) = cache.get(name) {
        return if identity.is_empty() {
            Err(Error::Pool)
        } else {
            Ok(identity.clone())
        };
    }
    let mut key = String::new();
    let mut position = source.root(&key);
    let mut identity = String::new();
    let mut first = true;

    loop {
        let Some(component) = parts.next() else {
            if identity.is_empty() {
                return Err(Error::Pool);
            }
            return Ok(identity);
        };

        if !first {
            key.push('/');
        }
        key.push_str(component);
        first = false;

        if let Some(found) = crate::identities::lookup(cache, &key) {
            position = source.cached_position(&key, &found);
            identity = found;
        } else {
            let (resolved, found) = match source.step(position, component, &key) {
                Ok(result) => result,
                Err(error) => return Err(Error::Read(error)),
            };
            position = resolved;
            identity = found;
            if !crate::identities::remember(cache, key.clone(), identity.clone()) {
                return Err(Error::Changed);
            }
        }
    }
}

/// Standard borrowed splitting, absolute-path classification, and error values.
pub trait Factory<E> {
    type Iter<'a>: Parts;

    fn parts<'a>(&self, name: &'a str) -> Self::Iter<'a>;
    fn absolute(&self, name: &str) -> bool;
    fn identity_error(&self, error: Error<E>) -> E;
    fn catalog_error(&self, unsupported: bool) -> E;
}
