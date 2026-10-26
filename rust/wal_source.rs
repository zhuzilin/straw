//! Replay descriptor ownership, buffered/plain transitions and error isolation.
//! File and standard-library buffer effects have separate lower contracts.
use crate::wal_device::Device;
#[cfg(straw_verus)]
use crate::wal_device::DeviceModel;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg(straw_verus)]
verus! {
    pub struct FileView {
        pub identity: int,
        pub bytes: Seq<u8>,
        pub cursor: nat,
        pub durable: nat,
    }
    pub type Loan = (int, FileView);
    pub trait FileModel: View<V = FileView> {
        open spec fn valid(&self) -> bool { true }
        #[verifier::prophetic]
        spec fn loan(&self) -> Loan;
    }
    pub open spec fn device(v: FileView, buffered: bool) -> crate::wal_device::DeviceView {
        crate::wal_device::DeviceView { identity: v.identity, bytes: v.bytes,
            cursor: v.cursor, durable: v.durable, buffered }
    }
}
#[cfg(not(straw_verus))]
pub trait FileModel {}
#[cfg(not(straw_verus))]
impl<T> FileModel for T {}

#[cfg_attr(straw_verus, verus_verify)]
pub trait File: FileModel + Sized {
    type Error;
    type Buffered: Buffer<File = Self, Error = Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self.valid(),
        ensures result.is_ok() ==> result->Ok_0 == self@.bytes.len(),
    ))]
    fn length(&self) -> Result<u64, Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self.valid(),
        ensures result.loan() == self.loan(), result.valid(), result@ == self@,
    ))]
    fn buffer(self, limit: u64, capacity: usize) -> Self::Buffered;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity,
            result.is_ok() ==> final(self)@.bytes == old(self)@.bytes
                && final(self)@.durable == old(self)@.durable,
            result.is_ok() ==> crate::wal_device::read_matches(old(self)@.bytes, old(self)@.cursor, length as nat, result->Ok_0@)
                && final(self)@.cursor == old(self)@.cursor + length,
    ))]
    fn read(&mut self, length: usize) -> Result<Vec<u8>, Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), end <= old(self)@.bytes.len(),
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity,
            result.is_ok() ==> final(self)@.bytes == old(self)@.bytes.take(end as int),
    ))]
    fn truncate(&mut self, end: u64) -> Result<(), Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity,
            result.is_ok() ==> final(self)@.bytes == old(self)@.bytes
                && final(self)@.durable == old(self)@.durable && final(self)@.cursor == offset,
    ))]
    fn seek(&mut self, offset: u64) -> Result<(), Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures result@ == crate::wal_device::digest(first@ + second@), result.len() == 32,
    ))]
    fn hash(first: &[u8], second: &[u8]) -> Vec<u8>;

    #[cfg_attr(straw_verus, verus_spec(
        requires self.valid(),
        ensures self.loan() == (self@.identity, self@),
    ))]
    fn finish(self);

    fn unavailable(message: &str) -> Self::Error;
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Buffer: FileModel + Sized {
    type Error;
    type File: FileModel;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self.valid(),
        ensures result.loan() == self.loan(), result.valid(), result@.identity == self@.identity, result@.bytes == self@.bytes,
            result@.durable == self@.durable,
    ))]
    fn into_file(self) -> Self::File;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity,
            result.is_ok() ==> final(self)@.bytes == old(self)@.bytes
                && final(self)@.durable == old(self)@.durable,
            result.is_ok() ==> crate::wal_device::read_matches(old(self)@.bytes, old(self)@.cursor, length as nat, result->Ok_0@)
                && final(self)@.cursor == old(self)@.cursor + length,
    ))]
    fn read(&mut self, length: usize) -> Result<Vec<u8>, Self::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Barrier<F: File>: Sized {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(file).valid(),
        ensures final(file).loan() == old(file).loan(), final(file).valid(), final(file)@.identity == old(file)@.identity,
            result.is_ok() ==> final(file)@.bytes == old(file)@.bytes
                && final(file)@.durable == final(file)@.bytes.len(),
    ))]
    fn sync(self, file: &mut F) -> Result<(), F::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
enum Input<F: File> {
    Buffered(F::Buffered),
    Plain(F),
}

#[cfg(straw_verus)]
verus! {
    impl<F: File> Input<F> {
        #[verifier::prophetic]
        closed spec fn loan(&self) -> Loan {
            match self { Self::Buffered(b) => b.loan(), Self::Plain(f) => f.loan() }
        }
        closed spec fn valid(&self) -> bool {
            match self { Self::Buffered(b) => b.valid(), Self::Plain(f) => f.valid() }
        }
    }
    impl<F: File> View for Input<F> {
        type V = crate::wal_device::DeviceView;
        closed spec fn view(&self) -> Self::V {
            match self { Self::Buffered(b) => device(b@, true), Self::Plain(f) => device(f@, false) }
        }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<F: File> Input<F> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self.valid(),
        ensures result.loan() == self.loan(), result.valid(), result@.identity == self@.identity, result@.bytes == self@.bytes,
            result@.durable == self@.durable,
            !self@.buffered ==> result@.cursor == self@.cursor,
    ))]
    fn into_file(self) -> F {
        match self {
            Self::Buffered(buffer) => buffer.into_file(),
            Self::Plain(file) => file,
        }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Source<F: File, S: Barrier<F>> {
    input: Option<Input<F>>,
    size: u64,
    sync: Option<S>,
    uncertain: bool,
}

#[cfg(straw_verus)]
verus! {
    impl<F: File, S: Barrier<F>> View for Source<F, S> {
        type V = crate::wal_device::DeviceView;
        closed spec fn view(&self) -> Self::V { self.input.unwrap()@ }
    }
    impl<F: File, S: Barrier<F>> crate::wal_device::DeviceModel for Source<F, S> {
        type Loan = Loan;
        #[verifier::prophetic]
        closed spec fn loan(&self) -> Loan { self.input.unwrap().loan() }
        closed spec fn valid(&self) -> bool {
            self.input.is_some() && self.input.unwrap().valid() && (!self.uncertain ==> self.size == self@.bytes.len())
        }
    }
    impl<F: File, S: Barrier<F>> Source<F, S> {
        pub closed spec fn available(&self) -> bool { !self.uncertain }
        pub closed spec fn has_barrier(&self) -> bool { self.sync.is_some() }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<F: File, S: Barrier<F>> Source<F, S> {
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires file.valid(),
        ensures result.is_ok() ==> result->Ok_0.valid() && result->Ok_0@ == device(file@, true)
            && result->Ok_0.available() && result->Ok_0.has_barrier(),
            result.is_ok() ==> result->Ok_0.loan() == file.loan(),
            result.is_err() ==> file.loan() == (file@.identity, file@),
    ))]
    pub fn from_file(file: F, sync: S) -> Result<Self, F::Error> {
        let size = match file.length() {
            Ok(size) => size,
            Err(error) => {
                file.finish();
                return Err(error);
            }
        };
        Ok(Self {
            input: Some(Input::Buffered(file.buffer(size, 0))),
            size,
            sync: Some(sync),
            uncertain: false,
        })
    }
    #[cfg_attr(straw_verus, verus_spec(
        requires self.valid(),
        ensures self.loan().0 == self@.identity,
            self.loan().1.identity == self@.identity,
            self.loan().1.bytes == self@.bytes, self.loan().1.durable == self@.durable,
            !self@.buffered ==> self.loan().1.cursor == self@.cursor,
    ))]
    pub fn finish(self) {
        self.input.unwrap().into_file().finish();
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl<F: File, S: Barrier<F>> Device for Source<F, S> {
    type Error = F::Error;

    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures result.is_ok() == self.available(),
    ))]
    fn length(&self) -> Result<u64, Self::Error> {
        if self.uncertain {
            return Err(F::unavailable("Uncertain replay device requires reopening"));
        }
        Ok(self.size)
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures final(self).available() == result.is_ok(),
            final(self).has_barrier() == old(self).has_barrier(),
    ))]
    fn read(&mut self, length: usize) -> Result<Vec<u8>, Self::Error> {
        self.length()?;
        self.uncertain = true;
        let input = &mut self.input;
        let bytes = match input.as_mut().unwrap() {
            Input::Buffered(reader) => reader.read(length)?,
            Input::Plain(file) => file.read(length)?,
        };
        self.uncertain = false;
        Ok(bytes)
    }

    fn hash(&self, first: &[u8], second: &[u8]) -> Vec<u8> {
        F::hash(first, second)
    }

    #[cfg_attr(straw_verus, verus_spec(
        ensures final(self).available() == old(self).available(),
            final(self).has_barrier() == old(self).has_barrier(),
    ))]
    fn discard_buffer(&mut self) {
        let input = &mut self.input;
        *input = Some(Input::Plain(input.take().unwrap().into_file()));
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures final(self).available() == result.is_ok(),
            final(self).has_barrier() == old(self).has_barrier(),
    ))]
    fn truncate(&mut self, end: u64) -> Result<(), Self::Error> {
        self.length()?;
        self.uncertain = true;
        let input = &mut self.input;
        let Input::Plain(file) = input.as_mut().unwrap() else {
            return Err(F::unavailable(
                "Replay must discard its buffer before truncating",
            ));
        };
        file.truncate(end)?;
        self.size = end;
        self.uncertain = false;
        Ok(())
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures final(self).has_barrier() == (!old(self).available() && old(self).has_barrier()),
            result.is_ok() ==> old(self).has_barrier() && final(self).available(),
            result.is_err() && old(self).has_barrier() ==> !final(self).available(),
    ))]
    fn sync(&mut self) -> Result<(), Self::Error> {
        self.length()?;
        let barrier = &mut self.sync;
        let Some(sync) = barrier.take() else {
            return Err(F::unavailable(
                "Replay attempted more than one durability barrier",
            ));
        };
        self.uncertain = true;
        let input = &mut self.input;
        let Input::Plain(file) = input.as_mut().unwrap() else {
            return Err(F::unavailable(
                "Replay must discard its buffer before synchronizing",
            ));
        };
        sync.sync(file)?;
        self.uncertain = false;
        Ok(())
    }

    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures final(self).available() == result.is_ok(),
            final(self).has_barrier() == old(self).has_barrier(),
    ))]
    fn seek(&mut self, offset: u64) -> Result<(), Self::Error> {
        self.length()?;
        self.uncertain = true;
        let slot = &mut self.input;
        let input = slot.take().unwrap();
        let buffered = matches!(input, Input::Buffered(_));
        let mut file = input.into_file();
        let outcome = file.seek(offset);
        *slot = Some(if buffered {
            let remaining = self.size.saturating_sub(offset);
            Input::Buffered(file.buffer(
                remaining,
                crate::replay::buffer_capacity(remaining) as usize,
            ))
        } else {
            Input::Plain(file)
        });
        outcome?;
        self.uncertain = false;
        Ok(())
    }
}
