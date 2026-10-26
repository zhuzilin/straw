//! Actual native WAL adapter. The I/O token is erased in ordinary builds.
use crate::file_primitives as raw;
#[cfg(straw_verus)]
use crate::wal_source::FileModel;
use std::marker::PhantomData;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg(straw_verus)]
verus! { pub trait DigestModel: View<V=Seq<u8>> {} }
#[cfg(not(straw_verus))]
pub trait DigestModel {}
#[cfg(not(straw_verus))]
impl<T> DigestModel for T {}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Digest: DigestModel + Sized {
    #[cfg_attr(straw_verus, verus_spec(result => ensures result@ == Seq::<u8>::empty()))]
    fn new() -> Self;
    #[cfg_attr(straw_verus, verus_spec(ensures final(self)@ == old(self)@ + bytes@))]
    fn update(&mut self, bytes: &[u8]);
    #[cfg_attr(straw_verus, verus_spec(result => ensures result@ == crate::wal_device::digest(self@), result.len() == 32))]
    fn finish(self) -> Vec<u8>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Library {
    type Error;
    type Digest: Digest;
    fn io(error: std::io::Error) -> Self::Error;
    fn unavailable(message: &str) -> Self::Error;
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct NativeFile<'a, L: Library> {
    file: &'a mut std::fs::File,
    #[cfg(straw_verus)]
    permission: &'a mut Tracked<raw::Permission>,
    library: PhantomData<L>,
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct NativeBuffer<'a, L: Library> {
    buffer: raw::Buffered<'a>,
    #[cfg(straw_verus)]
    permission: &'a mut Tracked<raw::Permission>,
    library: PhantomData<L>,
}
#[cfg(straw_verus)]
verus! {
    impl<L:Library> NativeFile<'_,L> {
        #[verifier::prophetic]
        pub closed spec fn returned_file(&self) -> std::fs::File { *final(self.file) }
        #[verifier::prophetic]
        pub closed spec fn returned_state(&self) -> crate::wal_source::FileView { final(self.permission)@@ }
    }
    impl<L:Library> View for NativeFile<'_,L> {
        type V=crate::wal_source::FileView;
        closed spec fn view(&self)->Self::V { self.permission@@ }
    }
    impl<L:Library> FileModel for NativeFile<'_,L> {
        #[verifier::prophetic]
        closed spec fn loan(&self) -> crate::wal_source::Loan { (raw::identity(self.returned_file()),self.returned_state()) }
        closed spec fn valid(&self)->bool { self@.identity == raw::identity(*self.file) }
    }
    impl<L:Library> View for NativeBuffer<'_,L> {
        type V=crate::wal_source::FileView;
        closed spec fn view(&self)->Self::V { self.permission@@ }
    }
    impl<L:Library> FileModel for NativeBuffer<'_,L> {
        #[verifier::prophetic]
        closed spec fn loan(&self) -> crate::wal_source::Loan { (raw::buffered_final_identity(self.buffer),final(self.permission)@@) }
        closed spec fn valid(&self)->bool { self@.identity == raw::buffered_identity(self.buffer) }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<'a, L: Library> NativeFile<'a, L> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires permission@@.identity == raw::identity(*file),
        ensures result.valid(), result@ == old(permission)@@,
            *final(file) == result.returned_file(),
            final(permission)@@ == result.returned_state(),
            result.loan() == (raw::identity(*final(file)), final(permission)@@),
    ))]
    pub fn new(
        file: &'a mut std::fs::File,
        #[cfg(straw_verus)] permission: &'a mut Tracked<raw::Permission>,
    ) -> Self {
        Self {
            file,
            #[cfg(straw_verus)]
            permission,
            library: PhantomData,
        }
    }
    #[cfg_attr(straw_verus, verus_spec(
        requires self.valid(),
        ensures self.returned_state() == self@,
            raw::identity(self.returned_file()) == self@.identity,
    ))]
    pub fn finish(self) {}
    // Callback wrappers are bound separately to the same file and token.
    #[cfg(not(straw_verus))]
    pub fn file(&self) -> &std::fs::File {
        self.file
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<'a, L: Library> crate::wal_source::File for NativeFile<'a, L> {
    type Error = L::Error;
    type Buffered = NativeBuffer<'a, L>;
    fn length(&self) -> Result<u64, Self::Error> {
        match raw::length(
            self.file,
            #[cfg(straw_verus)]
            self.permission,
        ) {
            Ok(length) => Ok(length),
            Err(error) => Err(L::io(error)),
        }
    }
    fn buffer(self, limit: u64, capacity: usize) -> Self::Buffered {
        let buffer = raw::buffer(
            self.file,
            limit,
            capacity,
            #[cfg(straw_verus)]
            self.permission,
        );
        NativeBuffer {
            buffer,
            #[cfg(straw_verus)]
            permission: self.permission,
            library: PhantomData,
        }
    }
    #[cfg_attr(straw_verus, verus_spec(ensures
        final(self).returned_file() == old(self).returned_file(),
        final(self).returned_state() == old(self).returned_state(),
    ))]
    fn read(&mut self, length: usize) -> Result<Vec<u8>, Self::Error> {
        let mut bytes = vec![0; length];
        match raw::read(
            self.file,
            &mut bytes,
            #[cfg(straw_verus)]
            self.permission,
        ) {
            Ok(()) => Ok(bytes),
            Err(error) => Err(L::io(error)),
        }
    }
    #[cfg_attr(straw_verus, verus_spec(ensures
        final(self).returned_file() == old(self).returned_file(),
        final(self).returned_state() == old(self).returned_state(),
    ))]
    fn truncate(&mut self, end: u64) -> Result<(), Self::Error> {
        match raw::truncate(
            self.file,
            end,
            #[cfg(straw_verus)]
            self.permission,
        ) {
            Ok(()) => Ok(()),
            Err(error) => Err(L::io(error)),
        }
    }
    #[cfg_attr(straw_verus, verus_spec(ensures
        final(self).returned_file() == old(self).returned_file(),
        final(self).returned_state() == old(self).returned_state(),
    ))]
    fn seek(&mut self, offset: u64) -> Result<(), Self::Error> {
        match raw::seek(
            self.file,
            offset,
            #[cfg(straw_verus)]
            self.permission,
        ) {
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

#[cfg_attr(straw_verus, verus_verify)]
impl<'a, L: Library> crate::wal_source::Buffer for NativeBuffer<'a, L> {
    type Error = L::Error;
    type File = NativeFile<'a, L>;
    fn into_file(self) -> Self::File {
        let file = raw::unbuffer(
            self.buffer,
            #[cfg(straw_verus)]
            self.permission,
        );
        NativeFile::new(
            file,
            #[cfg(straw_verus)]
            self.permission,
        )
    }
    fn read(&mut self, length: usize) -> Result<Vec<u8>, Self::Error> {
        let mut bytes = vec![0; length];
        match raw::read_buffered(
            &mut self.buffer,
            &mut bytes,
            #[cfg(straw_verus)]
            self.permission,
        ) {
            Ok(()) => Ok(bytes),
            Err(error) => Err(L::io(error)),
        }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<L: Library> crate::wal_sink::File for NativeFile<'_, L> {
    #[cfg_attr(straw_verus, verus_spec(ensures
        final(self).returned_file() == old(self).returned_file(),
        final(self).returned_state() == old(self).returned_state(),
    ))]
    fn position(&mut self) -> Result<u64, Self::Error> {
        match raw::position(
            self.file,
            #[cfg(straw_verus)]
            self.permission,
        ) {
            Ok(position) => Ok(position),
            Err(error) => Err(L::io(error)),
        }
    }
    #[cfg_attr(straw_verus, verus_spec(ensures
        final(self).returned_file() == old(self).returned_file(),
        final(self).returned_state() == old(self).returned_state(),
    ))]
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        match raw::write(
            self.file,
            bytes,
            #[cfg(straw_verus)]
            self.permission,
        ) {
            Ok(()) => Ok(()),
            Err(error) => Err(L::io(error)),
        }
    }
    #[cfg_attr(straw_verus, verus_spec(ensures
        final(self).returned_file() == old(self).returned_file(),
        final(self).returned_state() == old(self).returned_state(),
    ))]
    fn sync_all(&mut self) -> Result<(), Self::Error> {
        match raw::sync(
            self.file,
            #[cfg(straw_verus)]
            self.permission,
        ) {
            Ok(()) => Ok(()),
            Err(error) => Err(L::io(error)),
        }
    }
}
