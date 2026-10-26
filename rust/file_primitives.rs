//! Narrow standard I/O contracts. Mutable filesystem state belongs to a tracked
//! permission, never to the immutable logical value of a std File handle.
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Take, Write};

pub type Buffered<'a> = BufReader<Take<&'a mut File>>;

pub fn length(file: &File) -> std::io::Result<u64> {
    Ok(file.metadata()?.len())
}

pub fn buffer<'a>(file: &'a mut File, limit: u64, capacity: usize) -> Buffered<'a> {
    BufReader::with_capacity(capacity, file.take(limit))
}

pub fn unbuffer<'a>(buffer: Buffered<'a>) -> &'a mut File {
    buffer.into_inner().into_inner()
}

pub fn read(file: &mut File, bytes: &mut [u8]) -> std::io::Result<()> {
    file.read_exact(bytes)
}

pub fn read_buffered(file: &mut Buffered<'_>, bytes: &mut [u8]) -> std::io::Result<()> {
    file.read_exact(bytes)
}

pub fn truncate(file: &File, end: u64) -> std::io::Result<()> {
    file.set_len(end)
}

pub fn seek(file: &mut File, offset: u64) -> std::io::Result<u64> {
    file.seek(SeekFrom::Start(offset))
}

pub fn position(file: &mut File) -> std::io::Result<u64> {
    file.stream_position()
}

pub fn write(file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
    file.write_all(bytes)
}

pub fn sync(file: &File) -> std::io::Result<()> {
    file.sync_all()
}
