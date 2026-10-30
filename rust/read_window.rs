//! One bounded, disposable read window for an immutable extent.
//!
//! The bytes here are untrusted scratch. Callers authenticate each selected
//! frame and payload, and discard the entire window before a failed read retries.
use std::io::{self, Read, Seek, SeekFrom};

pub(crate) struct Window<R> {
    source: R,
    capacity: usize,
    end: u64,
    prefetch_end: u64,
    position: u64,
    start: u64,
    bytes: Vec<u8>,
}

impl<R: Read + Seek> Window<R> {
    pub(crate) fn new(source: R, capacity: usize, end: u64) -> Self {
        Self {
            source,
            capacity,
            end,
            prefetch_end: end,
            position: 0,
            start: 0,
            bytes: Vec::new(),
        }
    }
    pub(crate) fn covers(&self, start: u64, end: u64) -> bool {
        start >= self.start && end <= self.start.saturating_add(self.bytes.len() as u64)
    }
    pub(crate) fn prefetch_through(&mut self, end: u64) {
        self.prefetch_end = end.min(self.end);
    }
}

impl<R: Read + Seek> Read for Window<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let wanted = output
            .len()
            .min(self.end.saturating_sub(self.position) as usize);
        if wanted == 0 {
            return Ok(0);
        }
        let cached = self
            .position
            .checked_sub(self.start)
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n < self.bytes.len());
        if let Some(offset) = cached {
            let count = wanted.min(self.bytes.len() - offset);
            output[..count].copy_from_slice(&self.bytes[offset..offset + count]);
            self.position += count as u64;
            return Ok(count);
        }
        self.source.seek(SeekFrom::Start(self.position))?;
        if wanted >= self.capacity || self.position >= self.prefetch_end {
            let count = self.source.read(&mut output[..wanted])?;
            self.position += count as u64;
            return Ok(count);
        }
        let length = self
            .capacity
            .min(self.prefetch_end.saturating_sub(self.position) as usize);
        self.bytes.resize(length, 0);
        let count = self.source.read(&mut self.bytes)?;
        self.bytes.truncate(count);
        self.start = self.position;
        let count = count.min(wanted);
        output[..count].copy_from_slice(&self.bytes[..count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl<R: Read + Seek> Seek for Window<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::Current(n) => self.position.checked_add_signed(n),
            SeekFrom::End(n) => self.end.checked_add_signed(n),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid read window seek"))?;
        self.position = position;
        Ok(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Counted {
        inner: Cursor<Vec<u8>>,
        reads: usize,
    }
    impl Read for Counted {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            self.inner.read(bytes)
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.inner.seek(from)
        }
    }

    #[test]
    fn adjacent_reads_share_io_and_seek_preserves_window() -> io::Result<()> {
        let source = Counted {
            inner: Cursor::new((0..200).collect()),
            reads: 0,
        };
        let mut window = Window::new(source, 64, 150);
        for position in [10, 30, 50, 10] {
            window.seek(SeekFrom::Start(position))?;
            let mut bytes = [0; 10];
            window.read_exact(&mut bytes)?;
            assert_eq!(
                bytes.to_vec(),
                (position as u8..position as u8 + 10).collect::<Vec<_>>()
            );
        }
        assert_eq!(window.source.reads, 1);
        window.seek(SeekFrom::Start(130))?;
        let mut tail = Vec::new();
        window.read_to_end(&mut tail)?;
        assert_eq!(tail, (130..150).collect::<Vec<u8>>());
        assert_eq!(window.source.reads, 2);
        assert!(window.seek(SeekFrom::Current(-151)).is_err());
        Ok(())
    }

    #[test]
    fn large_read_bypasses_scratch_and_short_source_reaches_eof() -> io::Result<()> {
        let source = Counted {
            inner: Cursor::new(vec![9; 100]),
            reads: 0,
        };
        let mut window = Window::new(source, 16, 150);
        let mut bytes = [0; 80];
        window.read_exact(&mut bytes)?;
        assert_eq!(bytes, [9; 80]);
        assert!(window.bytes.is_empty());
        assert_eq!(window.source.reads, 1);
        assert_eq!(
            window.read_exact(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        Ok(())
    }
}
