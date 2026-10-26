//! Actual native WAL adapter. The I/O token is erased in ordinary builds.
use crate::file_primitives as raw;

use std::marker::PhantomData;

pub trait Digest: Sized {
    fn new() -> Self;

    fn update(&mut self, bytes: &[u8]);

    fn finish(self) -> Vec<u8>;
}

pub trait Library {
    type Error;
    type Digest: Digest;
    fn io(error: std::io::Error) -> Self::Error;
    fn unavailable(message: &str) -> Self::Error;
}

pub struct NativeFile<'a, L: Library> {
    file: &'a mut std::fs::File,

    library: PhantomData<L>,
}

pub struct NativeBuffer<'a, L: Library> {
    buffer: raw::Buffered<'a>,

    library: PhantomData<L>,
}

impl<'a, L: Library> NativeFile<'a, L> {
    pub fn new(file: &'a mut std::fs::File) -> Self {
        Self {
            file,

            library: PhantomData,
        }
    }

    pub fn finish(self) {}
    // Callback wrappers are bound separately to the same file and token.

    pub fn file(&self) -> &std::fs::File {
        self.file
    }
}

impl<'a, L: Library> crate::wal_source::File for NativeFile<'a, L> {
    type Error = L::Error;
    type Buffered = NativeBuffer<'a, L>;
    fn length(&self) -> Result<u64, Self::Error> {
        match raw::length(self.file) {
            Ok(length) => Ok(length),
            Err(error) => Err(L::io(error)),
        }
    }
    fn buffer(self, limit: u64, capacity: usize) -> Self::Buffered {
        let buffer = raw::buffer(self.file, limit, capacity);
        NativeBuffer {
            buffer,

            library: PhantomData,
        }
    }

    fn read(&mut self, length: usize) -> Result<Vec<u8>, Self::Error> {
        let mut bytes = vec![0; length];
        match raw::read(self.file, &mut bytes) {
            Ok(()) => Ok(bytes),
            Err(error) => Err(L::io(error)),
        }
    }

    fn truncate(&mut self, end: u64) -> Result<(), Self::Error> {
        match raw::truncate(self.file, end) {
            Ok(()) => Ok(()),
            Err(error) => Err(L::io(error)),
        }
    }

    fn seek(&mut self, offset: u64) -> Result<(), Self::Error> {
        match raw::seek(self.file, offset) {
            Ok(_) => Ok(()),
            Err(error) => Err(L::io(error)),
        }
    }
    fn hash(first: &[u8], second: &[u8]) -> Vec<u8> {
        let mut digest = L::Digest::new();
        digest.update(first);
        digest.update(second);
        digest.finish()
    }
    fn finish(self) {
        NativeFile::finish(self);
    }
    fn unavailable(message: &str) -> Self::Error {
        L::unavailable(message)
    }
}

impl<'a, L: Library> crate::wal_source::Buffer for NativeBuffer<'a, L> {
    type Error = L::Error;
    type File = NativeFile<'a, L>;
    fn into_file(self) -> Self::File {
        let file = raw::unbuffer(self.buffer);
        NativeFile::new(file)
    }
    fn read(&mut self, length: usize) -> Result<Vec<u8>, Self::Error> {
        let mut bytes = vec![0; length];
        match raw::read_buffered(&mut self.buffer, &mut bytes) {
            Ok(()) => Ok(bytes),
            Err(error) => Err(L::io(error)),
        }
    }
}

impl<L: Library> crate::wal_sink::File for NativeFile<'_, L> {
    fn position(&mut self) -> Result<u64, Self::Error> {
        match raw::position(self.file) {
            Ok(position) => Ok(position),
            Err(error) => Err(L::io(error)),
        }
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        match raw::write(self.file, bytes) {
            Ok(()) => Ok(()),
            Err(error) => Err(L::io(error)),
        }
    }

    fn sync_all(&mut self) -> Result<(), Self::Error> {
        match raw::sync(self.file) {
            Ok(()) => Ok(()),
            Err(error) => Err(L::io(error)),
        }
    }
}
