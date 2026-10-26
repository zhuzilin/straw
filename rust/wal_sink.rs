//! Actual append descriptor dispatch over file and fault-hook primitives.
use crate::framing::Kind;
use crate::wal_device::{Position, Writer};
#[cfg(straw_verus)]
use crate::{wal_device::DeviceModel, wal_source::FileModel};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub trait File: crate::wal_source::File {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity,
            result.is_ok() ==> final(self)@ == old(self)@ && result->Ok_0 == old(self)@.cursor,
    ))]
    fn position(&mut self) -> Result<u64, Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), old(self)@.cursor == old(self)@.bytes.len(),
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity,
            result.is_ok() ==> final(self)@.bytes == old(self)@.bytes + bytes@
                && final(self)@.cursor == final(self)@.bytes.len()
                && final(self)@.durable == old(self)@.durable,
    ))]
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity,
            result.is_ok() ==> final(self)@.bytes == old(self)@.bytes
                && final(self)@.cursor == old(self)@.cursor
                && final(self)@.durable == final(self)@.bytes.len(),
    ))]
    fn sync_all(&mut self) -> Result<(), Self::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Hooks<F: File> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(file).valid(),
        ensures final(file).loan() == old(file).loan(), final(file).valid(), final(file)@.identity == old(file)@.identity,
            result.is_ok() ==> final(file)@ == old(file)@,
    ))]
    fn point(&mut self, file: &mut F, point: &str) -> Result<(), F::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Appender<F: File, H: Hooks<F>> {
    file: F,
    kind: Kind,
    hooks: H,
    first: bool,
}

#[cfg(straw_verus)]
verus! {
    impl<F: File, H: Hooks<F>> View for Appender<F, H> {
        type V = crate::wal_device::DeviceView;
        closed spec fn view(&self) -> Self::V { crate::wal_source::device(self.file@, false) }
    }
    impl<F: File, H: Hooks<F>> crate::wal_device::DeviceModel for Appender<F, H> {
        type Loan = crate::wal_source::Loan;
        #[verifier::prophetic]
        closed spec fn loan(&self) -> Self::Loan { self.file.loan() }
        closed spec fn valid(&self) -> bool { self.file.valid() }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<F: File, H: Hooks<F>> Appender<F, H> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires file.valid(),
        ensures result.loan() == file.loan(), result.valid(), result@ == crate::wal_source::device(file@, false),
    ))]
    pub fn from_file(file: F, kind: Kind, hooks: H) -> Self {
        Self {
            file,
            kind,
            hooks,
            first: true,
        }
    }

    #[cfg_attr(straw_verus, verus_spec(
        requires self.valid(),
        ensures self.loan() == (self@.identity, crate::wal_source::FileView {
            identity:self@.identity, bytes:self@.bytes, cursor:self@.cursor, durable:self@.durable }),
    ))]
    pub fn finish(self) {
        self.file.finish();
    }

    /// Begin another append admission on the same borrowed descriptor.
    /// This resets only the once-per-append fault hook; it performs no I/O.
    #[cfg_attr(straw_verus, verus_spec(ensures final(self).loan() == old(self).loan(), final(self)@ == old(self)@, final(self).valid() == old(self).valid()))]
    pub fn restart(&mut self) {
        self.first = true;
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity,
            !final(self)@.buffered,
            result.is_ok() ==> final(self)@ == old(self)@,
    ))]
    pub fn point(&mut self, point: &str) -> Result<(), F::Error> {
        self.hooks.point(&mut self.file, point)
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<F: File, H: Hooks<F>> Writer for Appender<F, H> {
    type Error = F::Error;

    fn inspect(&mut self) -> Result<Position, Self::Error> {
        Ok(Position {
            size: self.file.length()?,
            cursor: self.file.position()?,
        })
    }

    fn hash(&self, first: &[u8], second: &[u8]) -> Vec<u8> {
        F::hash(first, second)
    }

    fn write(&mut self, part: &[u8]) -> Result<(), Self::Error> {
        if self.first {
            self.hooks.point(
                &mut self.file,
                match self.kind {
                    Kind::Queue => "before_journal_write",
                    Kind::Catalog => "before_storage_catalog_write",
                },
            )?;
            self.first = false;
        }
        self.file.write_all(part)?;
        self.hooks.point(
            &mut self.file,
            match self.kind {
                Kind::Queue => "after_journal_part",
                Kind::Catalog => "after_storage_catalog_part",
            },
        )
    }

    fn sync(&mut self) -> Result<(), Self::Error> {
        match self.kind {
            Kind::Queue => {
                self.hooks.point(&mut self.file, "before_journal_sync")?;
                self.hooks.point(&mut self.file, "before_file_sync")?;
                self.file.sync_all()?;
                self.hooks.point(&mut self.file, "after_journal_sync")
            }
            Kind::Catalog => {
                self.hooks
                    .point(&mut self.file, "before_storage_catalog_sync")?;
                self.file.sync_all()?;
                self.hooks
                    .point(&mut self.file, "after_storage_catalog_sync")
            }
        }
    }
}
