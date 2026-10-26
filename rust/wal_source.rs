//! Replay descriptor ownership, buffered/plain transitions and error isolation.
//! File and standard-library buffer effects have separate lower contracts.
use crate::wal_device::Device;

pub trait File: Sized {
    type Error;
    type Buffered: Buffer<File = Self, Error = Self::Error>;

    fn length(&self) -> Result<u64, Self::Error>;

    fn buffer(self, limit: u64, capacity: usize) -> Self::Buffered;

    fn read(&mut self, length: usize) -> Result<Vec<u8>, Self::Error>;

    fn truncate(&mut self, end: u64) -> Result<(), Self::Error>;

    fn seek(&mut self, offset: u64) -> Result<(), Self::Error>;

    fn hash(first: &[u8], second: &[u8]) -> Vec<u8>;

    fn finish(self);

    fn unavailable(message: &str) -> Self::Error;
}

pub trait Buffer: Sized {
    type Error;
    type File;

    fn into_file(self) -> Self::File;

    fn read(&mut self, length: usize) -> Result<Vec<u8>, Self::Error>;
}

pub trait Barrier<F: File>: Sized {
    fn sync(self, file: &mut F) -> Result<(), F::Error>;
}

enum Input<F: File> {
    Buffered(F::Buffered),
    Plain(F),
}

impl<F: File> Input<F> {
    fn into_file(self) -> F {
        match self {
            Self::Buffered(buffer) => buffer.into_file(),
            Self::Plain(file) => file,
        }
    }
}

pub struct Source<F: File, S: Barrier<F>> {
    input: Option<Input<F>>,
    size: u64,
    sync: Option<S>,
    uncertain: bool,
}

impl<F: File, S: Barrier<F>> Source<F, S> {
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

    pub fn finish(self) {
        self.input.unwrap().into_file().finish();
    }
}

impl<F: File, S: Barrier<F>> Device for Source<F, S> {
    type Error = F::Error;

    fn length(&self) -> Result<u64, Self::Error> {
        if self.uncertain {
            return Err(F::unavailable("Uncertain replay device requires reopening"));
        }
        Ok(self.size)
    }

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

    fn discard_buffer(&mut self) {
        let input = &mut self.input;
        *input = Some(Input::Plain(input.take().unwrap().into_file()));
    }

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
