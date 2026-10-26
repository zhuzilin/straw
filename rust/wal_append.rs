//! The byte writer shared by queue and catalog WAL appends.
//! Native descriptor and primitive-result correspondence is a separate boundary.
use crate::framing::{self, Kind};
use crate::wal::{self, AppendEffect};
use crate::wal_device::Writer;

pub enum Failure<E> {
    Device(E),
    Cursor,
    Boundary,
}

pub struct Appended {
    pub end: u64,
    pub sequence: u64,
}

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

    let mut state = wal::append_start();

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

    Ok(Appended {
        end,
        sequence: sequence + 1,
    })
}

struct Encoding {
    prefix: Vec<u8>,
    header: Vec<u8>,
    trailer: Vec<u8>,
}

fn encode<D: Writer>(device: &D, kind: Kind, sequence: u64, payload: &[u8]) -> Encoding {
    let prefix = framing::prefix(kind, sequence, payload.len() as u64);
    let header = framing::header(&prefix, &device.hash(&prefix, &[]));
    let trailer = framing::trailer(kind, &device.hash(&header, payload));

    Encoding {
        prefix,
        header,
        trailer,
    }
}
