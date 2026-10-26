//! The byte writer shared by queue and catalog WAL appends.
//! Native descriptor and primitive-result correspondence is a separate boundary.
use crate::framing::{self, Kind};
use crate::wal::{self, AppendEffect};
use crate::wal_device::Writer;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub enum Failure<E> {
    Device(E),
    Cursor,
    Boundary,
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Appended {
    pub end: u64,
    pub sequence: u64,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(device).valid(), !old(device)@.buffered,
    ensures final(device).loan() == old(device).loan(), final(device).valid(), final(device)@.identity == old(device)@.identity,
        !final(device)@.buffered,
        result.is_ok() ==> final(device)@.bytes.take(old(device)@.bytes.len() as int) == old(device)@.bytes,
        result.is_ok() ==> final(device)@.bytes.len() == old(device)@.bytes.len() + payload.len() + 96,
        result.is_ok() ==> final(device)@.cursor == final(device)@.bytes.len()
            && final(device)@.durable == final(device)@.bytes.len(),
        result.is_ok() ==> crate::wal_recovery::complete_frame(kind, final(device)@.bytes,
            old(device)@.bytes.len() as int, sequence as nat, payload@, final(device)@.bytes.len() as int),
        result.is_ok() ==> sequence < u64::MAX,
        result.is_ok() ==> old(device)@.bytes.len() == offset && old(device)@.cursor == offset,
        result.is_ok() ==> result->Ok_0.end == final(device)@.bytes.len()
            && result->Ok_0.sequence as int == sequence as int + 1,
        result.is_err() && result->Err_0 is Cursor ==> old(device)@.bytes.len() != offset || old(device)@.cursor != offset,
        result.is_err() && result->Err_0 is Boundary ==> sequence == u64::MAX
            || offset as int + payload.len() + 96 > u64::MAX as int,
        result.is_err() && !(result->Err_0 is Device) ==> final(device)@ == old(device)@,
))]
pub fn append<D: Writer>(
    device: &mut D,
    kind: Kind,
    sequence: u64,
    offset: u64,
    payload: &[u8],
) -> Result<Appended, Failure<D::Error>> {
    let position = match device.inspect() {
        Ok(position) => position,
        Err(error) => return Err(Failure::Device(error)),
    };
    if position.cursor != offset || position.size != offset {
        return Err(Failure::Cursor);
    }
    if sequence == u64::MAX {
        return Err(Failure::Boundary);
    }
    let end = match wal::frame_extent(offset, u64::MAX, payload.len() as u64) {
        wal::FrameExtent::Complete { end } => end,
        _ => return Err(Failure::Boundary),
    };
    let encoding = encode(device, kind, sequence, payload);
    let _prefix = encoding.prefix;
    let header = encoding.header;
    let trailer = encoding.trailer;
    #[cfg(straw_verus)]
    proof! {
        encoded_frame(old(device)@.bytes, kind, sequence, payload@, _prefix@, header@, trailer@);
    }
    let mut state = wal::append_start();
    #[cfg_attr(straw_verus, verus_spec(
        invariant device.loan() == old(device).loan(), device.valid(), state.completed <= 4, !state.failed,
            device@.identity == old(device)@.identity, !device@.buffered,
            device@.bytes == old(device)@.bytes + written(header@, payload@, trailer@, state.completed as nat),
            device@.cursor == device@.bytes.len(),
            state.completed == 4 ==> device@.durable == device@.bytes.len(),
        decreases 4 - state.completed,
    ))]
    while !wal::append_finished(&state) {
        let effect = wal::append_next(&state).unwrap();
        let outcome = match effect {
            AppendEffect::Header => device.write(&header),
            AppendEffect::Payload => device.write(payload),
            AppendEffect::Trailer => device.write(&trailer),
            AppendEffect::Sync => device.sync(),
        };
        state = wal::append_advance(state, outcome.is_ok());
        if let Err(error) = outcome {
            return Err(Failure::Device(error));
        }
    }
    #[cfg(straw_verus)]
    proof! {
        assert(state.completed == 4);
        assert(device@.bytes =~= old(device)@.bytes + header@ + payload@ + trailer@);
    }
    Ok(Appended {
        end,
        sequence: sequence + 1,
    })
}

#[cfg_attr(straw_verus, verus_verify)]
struct Encoding {
    prefix: Vec<u8>,
    header: Vec<u8>,
    trailer: Vec<u8>,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires payload.len() <= u64::MAX,
    ensures result.prefix.len() == 24,
        framing::range_matches(result.prefix@, 0, framing::magic_view(kind)),
        framing::le64(result.prefix@, 8) == sequence,
        framing::le64(result.prefix@, 16) == payload.len(),
        result.header@ == result.prefix@ + crate::wal_device::digest(result.prefix@),
        crate::wal_device::digest(result.prefix@).len() == 32,
        result.trailer@ == crate::wal_device::digest(result.header@ + payload@) + framing::end_view(kind),
        crate::wal_device::digest(result.header@ + payload@).len() == 32,
        result.header.len() == 56, result.trailer.len() == 40,
))]
fn encode<D: Writer>(device: &D, kind: Kind, sequence: u64, payload: &[u8]) -> Encoding {
    let prefix = framing::prefix(kind, sequence, payload.len() as u64);
    let header = framing::header(&prefix, &device.hash(&prefix, &[]));
    let trailer = framing::trailer(kind, &device.hash(&header, payload));
    #[cfg(straw_verus)]
    proof! { assert(prefix@ + Seq::<u8>::empty() =~= prefix@); }
    Encoding {
        prefix,
        header,
        trailer,
    }
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn written(header: Seq<u8>, payload: Seq<u8>, trailer: Seq<u8>, count: nat) -> Seq<u8> {
        if count == 0 { Seq::empty() }
        else if count == 1 { header }
        else if count == 2 { header + payload }
        else { header + payload + trailer }
    }
    pub proof fn encoded_frame(before: Seq<u8>, kind: Kind, sequence: u64, payload: Seq<u8>,
        prefix: Seq<u8>, header: Seq<u8>, trailer: Seq<u8>)
        requires prefix.len() == 24,
            framing::range_matches(prefix, 0, framing::magic_view(kind)),
            framing::le64(prefix, 8) == sequence, framing::le64(prefix, 16) == payload.len(),
            header == prefix + crate::wal_device::digest(prefix),
            crate::wal_device::digest(prefix).len() == 32,
            trailer == crate::wal_device::digest(header + payload) + framing::end_view(kind),
            crate::wal_device::digest(header + payload).len() == 32,
        ensures crate::wal_recovery::complete_frame(kind, before + header + payload + trailer,
            before.len() as int, sequence as nat, payload, (before + header + payload + trailer).len() as int),
            (before + header + payload + trailer).take(before.len() as int) == before,
    {
        framing::encoded_header_is_accepted(kind, prefix, crate::wal_device::digest(prefix), sequence, payload.len() as u64);
        let bytes = before + header + payload + trailer;
        assert(bytes.subrange(before.len() as int, before.len() as int + 24) =~= prefix);
        assert(bytes.subrange(before.len() as int, before.len() as int + 56) =~= header);
        assert(bytes.subrange(before.len() as int + 56, bytes.len() as int - 40) =~= payload);
        assert(bytes.take(before.len() as int) =~= before);
        assert(framing::le64(bytes, before.len() as int + 16) == payload.len());
        assert(framing::range_matches(bytes, bytes.len() as int - 40, crate::wal_device::digest(header + payload)));
        assert(framing::range_matches(bytes, bytes.len() as int - 8, framing::end_view(kind)));
    }
}
