//! Shared replay driver over explicit byte-I/O and application interfaces.

use crate::wal_device::{Device, Target};

pub fn finish<D: Device>(
    device: &mut D,
    end: u64,
    size: u64,
    writable: bool,
) -> Result<(), D::Error> {
    device.discard_buffer();
    if writable {
        if end != size {
            device.truncate(end)?;
        }
        device.sync()?;
    }
    device.seek(end)?;
    Ok(())
}

pub fn copy_range(bytes: &[u8], start: usize, length: usize) -> Vec<u8> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < length {
        result.push(bytes[start + i]);
        i += 1;
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Malformed {
    Cursor,
    PartialHeader,
    Header,
    FrameLength,
    Trailer,
    Sequence,
    ConfirmedPrefix,
}

pub enum Failure<I, A> {
    Device(I),
    Apply(A),
    Malformed(Malformed),
}

pub struct Boundary {
    pub end: u64,
    pub sequence: u64,
    pub incomplete: u64,
}

pub fn recover<D: Device, T: Target>(
    device: &mut D,
    target: &mut T,
    kind: crate::framing::Kind,
    start: u64,
    initial_sequence: u64,
    durable: u64,
    writable: bool,
) -> Result<Boundary, Failure<D::Error, T::Error>> {
    let size = match device.length() {
        Ok(size) => size,
        Err(e) => return Err(Failure::Device(e)),
    };
    if start > size || durable > size {
        return Err(Failure::Malformed(Malformed::Cursor));
    }
    if let Err(e) = device.seek(start) {
        return Err(Failure::Device(e));
    }
    let mut position = start;
    let mut sequence = initial_sequence;

    while position < size {
        let expected_sequence = sequence;

        let (payload, end) =
            match read_frame::<D, T::Error>(device, kind, position, size, expected_sequence) {
                Ok(Frame::Incomplete) => break,
                Ok(Frame::Complete { payload, end }) => (payload, end),
                Err(e) => return Err(e),
            };
        if sequence == u64::MAX {
            return Err(Failure::Malformed(Malformed::Sequence));
        }

        if let Err(e) = target.apply(&payload) {
            return Err(Failure::Apply(e));
        }

        sequence += 1;
        position = end;
    }
    if position < durable {
        return Err(Failure::Malformed(Malformed::ConfirmedPrefix));
    }
    if let Err(e) = finish(device, position, size, writable) {
        return Err(Failure::Device(e));
    }
    Ok(Boundary {
        end: position,
        sequence,
        incomplete: size - position,
    })
}

pub enum Frame {
    Incomplete,
    Complete { payload: Vec<u8>, end: u64 },
}

pub fn read_frame<D: Device, A>(
    device: &mut D,
    kind: crate::framing::Kind,
    start: u64,
    size: u64,
    sequence: u64,
) -> Result<Frame, Failure<D::Error, A>> {
    let remaining = size - start;
    if remaining < 56 {
        let partial = match device.read(remaining as usize) {
            Ok(bytes) => bytes,
            Err(e) => return Err(Failure::Device(e)),
        };
        if !crate::framing::partial_header_valid(kind, &partial) {
            return Err(Failure::Malformed(Malformed::PartialHeader));
        }
        return Ok(Frame::Incomplete);
    }
    let header = match device.read(56) {
        Ok(bytes) => bytes,
        Err(e) => return Err(Failure::Device(e)),
    };
    let prefix = copy_range(&header, 0, 24);
    let header_digest = device.hash(&prefix, &[]);

    let length = match crate::framing::parse_header(kind, &header, sequence, &header_digest) {
        Some(length) => length,
        None => return Err(Failure::Malformed(Malformed::Header)),
    };

    let end = match crate::wal::frame_extent(start, size, length) {
        crate::wal::FrameExtent::Malformed => {
            return Err(Failure::Malformed(Malformed::FrameLength));
        }
        crate::wal::FrameExtent::Truncated => return Ok(Frame::Incomplete),
        crate::wal::FrameExtent::Complete { end } => end,
    };
    if length > usize::MAX as u64 {
        return Err(Failure::Malformed(Malformed::FrameLength));
    }
    let payload = match device.read(length as usize) {
        Ok(bytes) => bytes,
        Err(e) => return Err(Failure::Device(e)),
    };
    let trailer = match device.read(40) {
        Ok(bytes) => bytes,
        Err(e) => return Err(Failure::Device(e)),
    };
    let frame_digest = device.hash(&header, &payload);

    if !crate::framing::trailer_valid(kind, &trailer, &frame_digest) {
        return Err(Failure::Malformed(Malformed::Trailer));
    }

    Ok(Frame::Complete { payload, end })
}
