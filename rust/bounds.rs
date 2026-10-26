//! Total arithmetic used by the storage and coordinator paths.
//! `straw_verus` enables contracts on these same function bodies; ordinary
//! Cargo builds erase the attributes and need no verification dependency.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures
        result.is_some() == (a as int + b as int <= u64::MAX as int),
        result.is_some() ==> result.unwrap() as int == a as int + b as int,
))]
pub fn add(a: u64, b: u64) -> Option<u64> {
    if b <= u64::MAX - a { Some(a + b) } else { None }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures
        result.is_some() == (base as int + 12 + metadata as int + payload as int <= u64::MAX as int),
        result.is_some() ==> result.unwrap() as int == base as int + 12 + metadata as int + payload as int,
))]
pub fn frame_end(base: u64, metadata: u64, payload: u64) -> Option<u64> {
    let header = add(base, 12)?;
    let body = add(header, metadata)?;
    add(body, payload)
}

/// Absolute frame position, only when the complete frame fits its extent and
/// both the extent and frame addresses are representable. The caller still
/// binds these lengths and offsets to authenticated metadata and a descriptor.
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures
        result.is_some() == (base as int + extent as int <= u64::MAX as int
            && relative as int + 12 + metadata as int + payload as int <= extent as int),
        result.is_some() ==> result.unwrap() as int == base as int + relative as int,
        result.is_some() ==> base <= result.unwrap(),
        result.is_some() ==> result.unwrap() as int + 12 + metadata as int + payload as int
            <= base as int + extent as int <= u64::MAX as int,
))]
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

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures
        increase ==> (result.is_some() == (value as int + amount as int <= u64::MAX as int)),
        !increase ==> (result.is_some() == (amount <= value)),
        result.is_some() ==> result.unwrap() as int ==
            if increase { value as int + amount as int } else { value as int - amount as int },
))]
pub fn adjust(value: u64, amount: u64, increase: bool) -> Option<u64> {
    if increase {
        add(value, amount)
    } else if amount <= value {
        Some(value - amount)
    } else {
        None
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures
        result <= count,
        result as int == if edge <= position { 0int }
            else if (edge as int) - (position as int) < (count as int) { edge as int - position as int }
            else { count as int },
))]
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
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures
        result.is_some() == (chunk > 0 && start <= stop && stop <= length),
        result.is_some() ==> result.unwrap().0 <= start <= stop <= result.unwrap().1 <= length,
        result.is_some() && start == stop ==> result.unwrap() == (start, stop),
        result.is_some() && start < stop ==> result.unwrap().0 as int ==
            start as int - start as int % chunk as int,
        result.is_some() && start < stop ==> result.unwrap().1 as int ==
            if length as int - ((stop as int - 1) - (stop as int - 1) % chunk as int) < chunk as int {
                length as int
            } else { (stop as int - 1) - (stop as int - 1) % chunk as int + chunk as int },
        result.is_some() ==> result.unwrap().1 as int - result.unwrap().0 as int <=
            stop as int - start as int + 2 * (chunk as int - 1),
        result.is_some() && stop as int == start as int + 1 ==>
            result.unwrap().1 as int - result.unwrap().0 as int <= chunk as int,
))]
pub fn authenticated_span(length: u64, start: u64, stop: u64, chunk: u64) -> Option<(u64, u64)> {
    if chunk == 0 || start > stop || stop > length {
        return None;
    }
    if start == stop {
        return Some((start, stop));
    }
    #[cfg(straw_verus)]
    proof! {
        vstd::arithmetic::div_mod::lemma_mod_decreases(start as nat, chunk as nat);
        vstd::arithmetic::div_mod::lemma_mod_decreases((stop - 1) as nat, chunk as nat);
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
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures
        result <= 1048576,
        result == if length < 1048576 { length } else { 1048576u64 },
        length > 0 ==> result > 0,
))]
pub fn verification_capacity(length: u64) -> u64 {
    if length < 1048576 { length } else { 1048576 }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures
        result.is_some() == (consumed < length && capacity > 0),
        result.is_some() ==> 0 < result.unwrap().0 <= capacity,
        result.is_some() ==> result.unwrap().1 as int == consumed as int + result.unwrap().0 as int,
        result.is_some() ==> consumed < result.unwrap().1 <= length,
        result.is_some() ==> result.unwrap().0 as int ==
            if (length as int) - (consumed as int) < (capacity as int) { length as int - consumed as int }
            else { capacity as int },
))]
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
