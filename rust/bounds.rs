//! Total arithmetic used by the storage and coordinator paths.

pub fn add(a: u64, b: u64) -> Option<u64> {
    if b <= u64::MAX - a { Some(a + b) } else { None }
}

pub fn frame_end(base: u64, metadata: u64, payload: u64) -> Option<u64> {
    let header = add(base, 12)?;
    let body = add(header, metadata)?;
    add(body, payload)
}

/// Absolute frame position, only when the complete frame fits its extent and
/// both the extent and frame addresses are representable. The caller still
/// binds these lengths and offsets to authenticated metadata and a descriptor.
pub fn frame_position(
    base: u64,
    extent: u64,
    relative: u64,
    metadata: u64,
    payload: u64,
) -> Option<u64> {
    add(base, extent)?;
    let end = frame_end(relative, metadata, payload)?;
    if end > extent {
        return None;
    }
    add(base, relative)
}

pub fn adjust(value: u64, amount: u64, increase: bool) -> Option<u64> {
    if increase {
        add(value, amount)
    } else if amount <= value {
        Some(value - amount)
    } else {
        None
    }
}

pub fn clip(edge: u64, position: u64, count: u64) -> u64 {
    if edge <= position {
        0
    } else if edge - position < count {
        edge - position
    } else {
        count
    }
}

/// Full checksum chunks touched by a half-open range, clamped to the payload.
/// The extra authenticated bytes are fewer than two chunks, independent of
/// the logical record size. Empty ranges authenticate no payload bytes.
pub fn authenticated_span(length: u64, start: u64, stop: u64, chunk: u64) -> Option<(u64, u64)> {
    if chunk == 0 || start > stop || stop > length {
        return None;
    }
    if start == stop {
        return Some((start, stop));
    }

    let first = start - start % chunk;
    let last = (stop - 1) - (stop - 1) % chunk;
    let end = if length - last < chunk {
        length
    } else {
        last + chunk
    };
    Some((first, end))
}

/// Payload/checksum verification scratch, independent of logical record size.
pub fn verification_capacity(length: u64) -> u64 {
    if length < 1048576 { length } else { 1048576 }
}

pub fn next_buffer(length: u64, consumed: u64, capacity: u64) -> Option<(u64, u64)> {
    if consumed >= length || capacity == 0 {
        return None;
    }
    let remaining = length - consumed;
    let count = if remaining < capacity {
        remaining
    } else {
        capacity
    };
    Some((count, consumed + count))
}
