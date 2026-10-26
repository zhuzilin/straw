//! Shared replay driver over explicit byte-I/O and application interfaces.
#[cfg(straw_verus)]
use crate::wal_device::digest;
use crate::wal_device::{Device, Target};
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(device).valid(), end <= old(device)@.bytes.len(), size == old(device)@.bytes.len(),
    ensures final(device).loan() == old(device).loan(), final(device).valid(), final(device)@.identity == old(device)@.identity,
        result.is_ok() ==> final(device)@.bytes == old(device)@.bytes || final(device)@.bytes == old(device)@.bytes.take(end as int),
        result.is_ok() ==> final(device)@.cursor == end && !final(device)@.buffered,
        result.is_ok() && writable ==> final(device)@.bytes == old(device)@.bytes.take(end as int),
        result.is_ok() && writable ==> final(device)@.durable == end,
        result.is_ok() && !writable ==> final(device)@.bytes == old(device)@.bytes,
))]
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

#[cfg_attr(straw_verus, verus_spec(result =>
    requires start <= bytes.len(), length <= bytes.len() - start,
    ensures result@ == bytes@.subrange(start as int, start as int + length),
))]
pub fn copy_range(bytes: &[u8], start: usize, length: usize) -> Vec<u8> {
    let mut result = Vec::new();
    let mut i = 0;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= length, start <= bytes.len(), length <= bytes.len() - start,
            result@ == bytes@.subrange(start as int, start as int + i),
        decreases length - i,
    ))]
    while i < length {
        result.push(bytes[start + i]);
        i += 1;
    }
    result
}

#[cfg_attr(straw_verus, verus_verify)]
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

#[cfg_attr(straw_verus, verus_verify)]
pub enum Failure<I, A> {
    Device(I),
    Apply(A),
    Malformed(Malformed),
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Boundary {
    pub end: u64,
    pub sequence: u64,
    pub incomplete: u64,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(device).valid(), old(target).valid(), old(target).complete(),
    ensures final(device).loan() == old(device).loan(), final(device).valid(), final(target).valid(), final(target).context() == old(target).context(),
        final(target).loan() == old(target).loan(),
        result.is_ok() ==> final(target).complete(),
        final(device)@.identity == old(device)@.identity,
        result.is_ok() ==> final(device)@.cursor == result->Ok_0.end && !final(device)@.buffered,
        result.is_ok() ==> durable <= result->Ok_0.end <= old(device)@.bytes.len(),
        result.is_ok() ==> result->Ok_0.incomplete as int == old(device)@.bytes.len() - result->Ok_0.end,
        result.is_ok() ==> result->Ok_0.sequence >= initial_sequence,
        result.is_ok() ==> final(target)@.len() == old(target)@.len() + result->Ok_0.sequence - initial_sequence,
        result.is_ok() ==> replayed(kind, old(device)@.bytes, start as int, initial_sequence as nat,
            old(target)@, final(target)@),
        result.is_ok() ==> result->Ok_0.end as int == run_end(start as int,
            final(target)@.subrange(old(target)@.len() as int, final(target)@.len() as int)),
        result.is_ok() ==> incomplete_tail(kind, old(device)@.bytes, result->Ok_0.end as int, result->Ok_0.sequence as nat),
        result.is_ok() && writable ==> final(device)@.bytes == old(device)@.bytes.take(result->Ok_0.end as int),
        result.is_ok() && writable ==> final(device)@.durable == result->Ok_0.end,
        result.is_ok() && !writable ==> final(device)@.bytes == old(device)@.bytes,
        match result {
            Err(Failure::Device(_)) => replayed(kind, old(device)@.bytes, start as int, initial_sequence as nat,
                old(target)@, final(target)@),
            Err(Failure::Malformed(reason)) => final(device)@.bytes == old(device)@.bytes
                && final(device)@.durable == old(device)@.durable
                && recovery_malformed(kind, old(device)@.bytes, start as int, initial_sequence as nat,
                    durable as int, old(target)@, final(target)@, reason),
            _ => true,
        },
))]
pub fn recover<D: Device, T: Target>(
    device: &mut D,
    target: &mut T,
    kind: crate::framing::Kind,
    start: u64,
    initial_sequence: u64,
    durable: u64,
    writable: bool,
) -> Result<Boundary, Failure<D::Error, T::Error>> {
    #[cfg(straw_verus)]
    proof! { empty_run(kind, device@.bytes, start as int, initial_sequence as nat, target@); }
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
    #[cfg_attr(straw_verus, verus_spec(
        invariant device.loan() == old(device).loan(), start <= position <= size, initial_sequence <= sequence, device.valid(), target.valid(), target.complete(),
            target.context() == old(target).context(),
            target.loan() == old(target).loan(),
            device@.identity == old(device)@.identity,
            device@.bytes == old(device)@.bytes, device@.durable == old(device)@.durable,
            size == device@.bytes.len(), durable <= size,
            target@.len() == old(target)@.len() + sequence - initial_sequence,
            replayed(kind, device@.bytes, start as int, initial_sequence as nat, old(target)@, target@),
            position as int == run_end(start as int, target@.subrange(old(target)@.len() as int, target@.len() as int)),
        invariant_except_break device@.cursor == position,
        ensures incomplete_tail(kind, device@.bytes, position as int, sequence as nat),
        decreases size - position,
    ))]
    while position < size {
        let expected_sequence = sequence;
        #[cfg(straw_verus)]
        proof! { assert(expected_sequence == sequence); }
        let (payload, end) =
            match read_frame::<D, T::Error>(device, kind, position, size, expected_sequence) {
                Ok(Frame::Incomplete) => break,
                Ok(Frame::Complete { payload, end }) => (payload, end),
                Err(e) => return Err(e),
            };
        if sequence == u64::MAX {
            #[cfg(straw_verus)]
            proof! {
                assert(exists|p: Seq<u8>, e: int| #[trigger] complete_frame(kind, device@.bytes, position as int,
                    sequence as nat, p, e)) by { assert(complete_frame(kind, device@.bytes, position as int,
                        sequence as nat, payload@, end as int)); }
            }
            return Err(Failure::Malformed(Malformed::Sequence));
        }
        #[cfg(straw_verus)]
        proof! {
            extend_run(kind, device@.bytes, start as int, initial_sequence as nat,
                old(target)@, target@, position as int, sequence as nat, payload@, end as int);
        }
        if let Err(e) = target.apply(&payload) {
            #[cfg(straw_verus)]
            proof! { assert(target.context() == old(target).context()); }
            return Err(Failure::Apply(e));
        }
        #[cfg(straw_verus)]
        proof! { assert(target.context() == old(target).context()); }
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

#[cfg_attr(straw_verus, verus_verify)]
pub enum Frame {
    Incomplete,
    Complete { payload: Vec<u8>, end: u64 },
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(device).valid(), start <= size, size == old(device)@.bytes.len(), old(device)@.cursor == start,
    ensures final(device).loan() == old(device).loan(), final(device).valid(), final(device)@.identity == old(device)@.identity,
        format_result(kind, old(device)@.bytes, start as int, sequence, result),
        result.is_ok() ==> final(device)@.bytes == old(device)@.bytes,
        result.is_ok() ==> final(device)@.durable == old(device)@.durable,
        final(device)@.buffered == old(device)@.buffered,
        match result {
            Ok(Frame::Complete { payload, end }) => start < end <= size
                && end as int == start as int + 96 + payload.len()
                && final(device)@.cursor == end,
            Err(Failure::Malformed(_)) => final(device)@.bytes == old(device)@.bytes
                && final(device)@.durable == old(device)@.durable,
            _ => true,
        },
))]
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
    #[cfg(straw_verus)]
    proof! {
        assert(prefix@ =~= device@.bytes.subrange(start as int, start as int + 24));
        assert(prefix@ + Seq::<u8>::empty() =~= prefix@);
        assert(crate::framing::le64(header@, 16) == crate::framing::le64(device@.bytes, start as int + 16));
    }
    let length = match crate::framing::parse_header(kind, &header, sequence, &header_digest) {
        Some(length) => length,
        None => return Err(Failure::Malformed(Malformed::Header)),
    };
    #[cfg(straw_verus)]
    proof! {
        assert(crate::framing::header_valid(kind, device@.bytes.subrange(start as int, start as int + 56),
            sequence, digest(device@.bytes.subrange(start as int, start as int + 24))));
    }
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
    #[cfg(straw_verus)]
    proof! {
        assert(frame_digest@ == digest(header@ + payload@));
        assert(payload@ == device@.bytes.subrange(start as int + 56, end as int - 40));
        assert(trailer@ == device@.bytes.subrange(end as int - 40, end as int));
        assert(crate::framing::range_matches(trailer@, 0, frame_digest@)
            == crate::framing::range_matches(device@.bytes, end as int - 40, frame_digest@));
        assert(crate::framing::range_matches(trailer@, 32, crate::framing::end_view(kind))
            == crate::framing::range_matches(device@.bytes, end as int - 8, crate::framing::end_view(kind)));
    }
    if !crate::framing::trailer_valid(kind, &trailer, &frame_digest) {
        return Err(Failure::Malformed(Malformed::Trailer));
    }
    #[cfg(straw_verus)]
    proof! {
        assert(crate::framing::range_matches(device@.bytes, end as int - 40, frame_digest@));
        assert(crate::framing::range_matches(device@.bytes, end as int - 8, crate::framing::end_view(kind)));
    }
    Ok(Frame::Complete { payload, end })
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn format_result<I, A>(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: u64, result: Result<Frame, Failure<I, A>>) -> bool {
        match result {
            Ok(Frame::Complete { payload, end }) => complete_frame(kind, bytes, start, sequence as nat, payload@, end as int),
            Ok(Frame::Incomplete) => incomplete_tail(kind, bytes, start, sequence as nat),
            Err(Failure::Malformed(reason)) => malformed_at(kind, bytes, start, sequence, reason),
            Err(Failure::Device(_)) => true,
            Err(Failure::Apply(_)) => false,
        }
    }
    pub proof fn complete_decodes<I, A>(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: u64, payload: Seq<u8>, end: int, result: Result<Frame, Failure<I, A>>)
        requires complete_frame(kind, bytes, start, sequence as nat, payload, end),
            bytes.len() <= u64::MAX, payload.len() <= usize::MAX,
            format_result(kind, bytes, start, sequence, result),
        ensures match result {
            Ok(Frame::Complete { payload: decoded, end: boundary }) => decoded@ == payload && boundary == end,
            Err(Failure::Device(_)) => true,
            _ => false,
        },
    {
        complete_not_malformed(kind, bytes, start, sequence, payload, end, Malformed::Header);
        if let Err(Failure::Malformed(reason)) = result {
            complete_not_malformed(kind, bytes, start, sequence, payload, end, reason);
        }
    }
    pub proof fn incomplete_decodes<I, A>(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: u64, result: Result<Frame, Failure<I, A>>)
        requires incomplete_tail(kind, bytes, start, sequence as nat), bytes.len() <= u64::MAX,
            format_result(kind, bytes, start, sequence, result),
        ensures match result { Ok(Frame::Incomplete) => true, Err(Failure::Device(_)) => true, _ => false },
    {
        match result {
            Ok(Frame::Complete { payload, end }) => {
                // This branch is impossible independently of usize allocation bounds.
                assert(start + 56 <= bytes.len());
                assert(start + crate::framing::le64(bytes, start + 16) as int + 96 <= bytes.len());
            },
            Err(Failure::Malformed(reason)) => incomplete_not_malformed(kind, bytes, start, sequence, reason),
            _ => {},
        }
    }
    pub open spec fn recovery_malformed(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        initial_sequence: nat, floor: int, initial: Seq<Seq<u8>>, current: Seq<Seq<u8>>, reason: Malformed) -> bool {
        if reason is Cursor { current == initial && (start > bytes.len() || floor > bytes.len()) }
        else {
            let position = run_end(start, current.subrange(initial.len() as int, current.len() as int));
            let sequence = initial_sequence + current.len() - initial.len();
            &&& 0 <= start <= bytes.len() && floor <= bytes.len()
            &&& replayed(kind, bytes, start, initial_sequence, initial, current)
            &&& initial_sequence <= sequence <= u64::MAX
            &&& match reason {
                Malformed::Sequence => sequence == u64::MAX
                    && exists|payload: Seq<u8>, end: int| #[trigger] complete_frame(kind, bytes, position, sequence as nat, payload, end),
                Malformed::ConfirmedPrefix => position < floor && incomplete_tail(kind, bytes, position, sequence as nat),
                _ => malformed_at(kind, bytes, position, sequence as u64, reason),
            }
        }
    }
    pub open spec fn header_at(kind: crate::framing::Kind, bytes: Seq<u8>, start: int, sequence: u64) -> bool {
        0 <= start && start + 56 <= bytes.len()
        && crate::framing::header_valid(kind, bytes.subrange(start, start + 56), sequence,
            digest(bytes.subrange(start, start + 24)))
    }
    /// Exact reasons for a format rejection. Device/target failures are separate.
    pub open spec fn malformed_at(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: u64, reason: Malformed) -> bool {
        let length = crate::framing::le64(bytes, start + 16) as int;
        let end = start + length + 96;
        match reason {
            Malformed::PartialHeader => 0 <= start <= bytes.len() && bytes.len() - start < 56
                && !(forall|i: int| 0 <= i < bytes.len() - start && i < 8 ==>
                    bytes[start + i] == crate::framing::magic_view(kind)[i]),
            Malformed::Header => 0 <= start && start + 56 <= bytes.len()
                && !header_at(kind, bytes, start, sequence),
            Malformed::FrameLength => header_at(kind, bytes, start, sequence)
                && (length + 96 > u64::MAX as int || (end <= bytes.len() && length > usize::MAX as int)),
            Malformed::Trailer => header_at(kind, bytes, start, sequence)
                && length + 96 <= u64::MAX as int && end <= bytes.len() && length <= usize::MAX as int
                && !(crate::framing::range_matches(bytes, end - 40,
                    digest(bytes.subrange(start, start + 56) + bytes.subrange(start + 56, end - 40)))
                    && crate::framing::range_matches(bytes, end - 8, crate::framing::end_view(kind))),
            _ => false,
        }
    }
    pub proof fn complete_not_malformed(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: u64, payload: Seq<u8>, end: int, reason: Malformed)
        requires complete_frame(kind, bytes, start, sequence as nat, payload, end),
            bytes.len() <= u64::MAX, payload.len() <= usize::MAX,
        ensures !malformed_at(kind, bytes, start, sequence, reason),
            !incomplete_tail(kind, bytes, start, sequence as nat),
    {}
    pub proof fn incomplete_not_malformed(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: u64, reason: Malformed)
        requires incomplete_tail(kind, bytes, start, sequence as nat),
        ensures !malformed_at(kind, bytes, start, sequence, reason),
    {}
    pub open spec fn complete_frame(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: nat, payload: Seq<u8>, end: int) -> bool {
        &&& 0 <= start && end == start + 96 + payload.len() && end <= bytes.len()
        &&& sequence <= u64::MAX
        &&& crate::framing::header_valid(kind, bytes.subrange(start, start + 56), sequence as u64,
                digest(bytes.subrange(start, start + 24)))
        &&& crate::framing::le64(bytes, start + 16) as int == payload.len()
        &&& payload == bytes.subrange(start + 56, end - 40)
        &&& crate::framing::range_matches(bytes, end - 40,
                digest(bytes.subrange(start, start + 56) + payload))
        &&& crate::framing::range_matches(bytes, end - 8, crate::framing::end_view(kind))
    }
    pub open spec fn incomplete_tail(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: nat) -> bool {
        &&& 0 <= start <= bytes.len()
        &&& sequence <= u64::MAX
        &&& if bytes.len() - start < 56 {
            forall|i: int| 0 <= i < bytes.len() - start && i < 8 ==>
                bytes[start + i] == crate::framing::magic_view(kind)[i]
        } else {
            &&& crate::framing::header_valid(kind, bytes.subrange(start, start + 56), sequence as u64,
                    digest(bytes.subrange(start, start + 24)))
            &&& crate::framing::le64(bytes, start + 16) as int + 96 <= u64::MAX
            &&& start + crate::framing::le64(bytes, start + 16) as int + 96 > bytes.len()
        }
    }
    #[verifier::opaque]
    pub open spec fn run_end(start: int, records: Seq<Seq<u8>>) -> int
        decreases records.len(),
    {
        if records.len() == 0 { start }
        else { run_end(start, records.drop_last()) + 96 + records.last().len() }
    }
    pub closed spec fn valid_run(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: nat, records: Seq<Seq<u8>>) -> bool
        decreases records.len(),
    {
        if records.len() == 0 { true }
        else {
            valid_run(kind, bytes, start, sequence, records.drop_last())
            && complete_frame(kind, bytes, run_end(start, records.drop_last()),
                (sequence + records.len() - 1) as nat, records.last(), run_end(start, records))
        }
    }
    pub open spec fn replayed(kind: crate::framing::Kind, bytes: Seq<u8>, start: int, sequence: nat,
        initial: Seq<Seq<u8>>, current: Seq<Seq<u8>>) -> bool {
        initial.len() <= current.len() && current.take(initial.len() as int) == initial
        && valid_run(kind, bytes, start, sequence, current.subrange(initial.len() as int, current.len() as int))
    }
    pub open spec fn valid_log(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: nat, records: Seq<Seq<u8>>) -> bool {
        0 <= start <= bytes.len() <= u64::MAX
        && sequence + records.len() <= u64::MAX
        && valid_run(kind, bytes, start, sequence, records)
        && incomplete_tail(kind, bytes, run_end(start, records), sequence + records.len())
        && forall|i: int| 0 <= i < records.len() ==> #[trigger] records[i].len() <= usize::MAX
    }
    pub proof fn frame_unique(kind: crate::framing::Kind, bytes: Seq<u8>, start: int, sequence: nat,
        first: Seq<u8>, first_end: int, second: Seq<u8>, second_end: int)
        requires complete_frame(kind, bytes, start, sequence, first, first_end),
            complete_frame(kind, bytes, start, sequence, second, second_end),
        ensures first == second, first_end == second_end,
    {}
    pub proof fn complete_excludes_incomplete(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: nat, payload: Seq<u8>, end: int)
        requires complete_frame(kind, bytes, start, sequence, payload, end),
        ensures !incomplete_tail(kind, bytes, start, sequence),
    {}
    pub proof fn run_prefix(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: nat, records: Seq<Seq<u8>>, count: nat)
        requires valid_run(kind, bytes, start, sequence, records), count <= records.len(),
        ensures valid_run(kind, bytes, start, sequence, records.take(count as int)),
        decreases records.len(),
    {
        reveal(valid_run);
        if count < records.len() {
            run_prefix(kind, bytes, start, sequence, records.drop_last(), count);
            assert(records.drop_last().take(count as int) == records.take(count as int));
        } else { assert(records.take(count as int) == records); }
    }
    pub proof fn run_record(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: nat, records: Seq<Seq<u8>>, index: nat)
        requires valid_run(kind, bytes, start, sequence, records), index < records.len(),
        ensures complete_frame(kind, bytes, run_end(start, records.take(index as int)), sequence + index,
            records[index as int], run_end(start, records.take(index as int + 1))),
    {
        run_prefix(kind, bytes, start, sequence, records, index + 1);
        assert(records.take(index as int + 1).drop_last() == records.take(index as int));
        reveal(valid_run);
    }
    pub proof fn equal_runs(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: nat, first: Seq<Seq<u8>>, second: Seq<Seq<u8>>)
        requires valid_run(kind, bytes, start, sequence, first),
            valid_run(kind, bytes, start, sequence, second), first.len() == second.len(),
        ensures first == second,
        decreases first.len(),
    {
        if first.len() > 0 {
            reveal(valid_run);
            equal_runs(kind, bytes, start, sequence, first.drop_last(), second.drop_last());
            frame_unique(kind, bytes, run_end(start, first.drop_last()), (sequence + first.len() - 1) as nat,
                first.last(), run_end(start, first), second.last(), run_end(start, second));
        }
        assert forall|i: int| 0 <= i < first.len() implies first[i] == second[i] by {
            if i + 1 < first.len() {
                assert(first[i] == first.drop_last()[i]);
                assert(second[i] == second.drop_last()[i]);
            } else {
                assert(first[i] == first.last());
                assert(second[i] == second.last());
            }
        }
        assert(first =~= second);
    }
    pub proof fn shorter_run_is_prefix(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: nat, short: Seq<Seq<u8>>, long: Seq<Seq<u8>>)
        requires valid_run(kind, bytes, start, sequence, short),
            valid_run(kind, bytes, start, sequence, long), short.len() <= long.len(),
        ensures short == long.take(short.len() as int),
    {
        run_prefix(kind, bytes, start, sequence, long, short.len());
        equal_runs(kind, bytes, start, sequence, short, long.take(short.len() as int));
    }
    pub proof fn accepted_run_is_prefix(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: nat, records: Seq<Seq<u8>>, accepted: Seq<Seq<u8>>)
        requires valid_log(kind, bytes, start, sequence, records),
            valid_run(kind, bytes, start, sequence, accepted),
        ensures accepted.len() <= records.len(), accepted == records.take(accepted.len() as int),
    {
        if accepted.len() > records.len() {
            shorter_run_is_prefix(kind, bytes, start, sequence, records, accepted);
            run_record(kind, bytes, start, sequence, accepted, records.len());
            complete_excludes_incomplete(kind, bytes, run_end(start, records), sequence + records.len(),
                accepted[records.len() as int], run_end(start, accepted.take(records.len() as int + 1)));
        } else { shorter_run_is_prefix(kind, bytes, start, sequence, accepted, records); }
    }
    pub proof fn legal_log_has_no_format_error(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: nat, floor: int, records: Seq<Seq<u8>>, initial: Seq<Seq<u8>>, current: Seq<Seq<u8>>, reason: Malformed)
        requires valid_log(kind, bytes, start, sequence, records), floor <= run_end(start, records),
        ensures !recovery_malformed(kind, bytes, start, sequence, floor, initial, current, reason),
    {
        if recovery_malformed(kind, bytes, start, sequence, floor, initial, current, reason) {
            if !(reason is Cursor) {
                let accepted = current.subrange(initial.len() as int, current.len() as int);
                accepted_run_is_prefix(kind, bytes, start, sequence, records, accepted);
                let position = run_end(start, accepted);
                let next = (sequence + accepted.len()) as u64;
                if accepted.len() < records.len() {
                    run_record(kind, bytes, start, sequence, records, accepted.len());
                    complete_not_malformed(kind, bytes, position, next, records[accepted.len() as int],
                        run_end(start, records.take(accepted.len() as int + 1)), reason);
                } else {
                    assert(accepted == records);
                    incomplete_not_malformed(kind, bytes, position, next, reason);
                    if reason is Sequence {
                        let (payload, end) = choose|payload: Seq<u8>, end: int|
                            complete_frame(kind, bytes, position, next as nat, payload, end);
                        complete_excludes_incomplete(kind, bytes, position, next as nat, payload, end);
                    }
                }
            }
        }
    }
    pub proof fn legal_log_recovers_exactly(kind: crate::framing::Kind, bytes: Seq<u8>, start: int,
        sequence: nat, records: Seq<Seq<u8>>, initial: Seq<Seq<u8>>, current: Seq<Seq<u8>>)
        requires valid_log(kind, bytes, start, sequence, records),
            replayed(kind, bytes, start, sequence, initial, current),
            incomplete_tail(kind, bytes, run_end(start, current.subrange(initial.len() as int, current.len() as int)),
                (sequence + current.len() - initial.len()) as nat),
        ensures current == initial + records,
    {
        let accepted = current.subrange(initial.len() as int, current.len() as int);
        accepted_run_is_prefix(kind, bytes, start, sequence, records, accepted);
        if accepted.len() < records.len() {
            run_record(kind, bytes, start, sequence, records, accepted.len());
            complete_excludes_incomplete(kind, bytes, run_end(start, accepted), sequence + accepted.len(),
                records[accepted.len() as int], run_end(start, records.take(accepted.len() as int + 1)));
        }
        assert(accepted == records);
        assert(current =~= initial + records);
    }
    pub proof fn empty_run(kind: crate::framing::Kind, bytes: Seq<u8>, start: int, sequence: nat,
        records: Seq<Seq<u8>>)
        ensures replayed(kind, bytes, start, sequence, records, records),
            run_end(start, records.subrange(records.len() as int, records.len() as int)) == start,
    {
        reveal(valid_run);
        reveal(run_end);
        assert(records.take(records.len() as int) =~= records);
    }
    pub proof fn extend_run(kind: crate::framing::Kind, bytes: Seq<u8>, start: int, initial_sequence: nat,
        initial: Seq<Seq<u8>>, records: Seq<Seq<u8>>, position: int, sequence: nat, payload: Seq<u8>, end: int)
        requires replayed(kind, bytes, start, initial_sequence, initial, records),
            position == run_end(start, records.subrange(initial.len() as int, records.len() as int)),
            sequence == initial_sequence + records.len() - initial.len(),
            complete_frame(kind, bytes, position, sequence, payload, end),
        ensures replayed(kind, bytes, start, initial_sequence, initial, records.push(payload)),
            end == run_end(start, records.push(payload).subrange(initial.len() as int, records.len() as int + 1)),
    {
        let prior = records.subrange(initial.len() as int, records.len() as int);
        let next = records.push(payload).subrange(initial.len() as int, records.len() as int + 1);
        assert(records.push(payload).take(initial.len() as int) =~= initial);
        assert(next =~= prior.push(payload));
        assert(next.drop_last() =~= prior);
        reveal(valid_run);
        reveal(run_end);
    }
}
