//! Queue/catalog wire framing. Digest computation and JSON remain adapters;
//! these are the actual byte checks used by both production replay paths.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy)]
pub enum Kind {
    Queue,
    Catalog,
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result@ == magic_view(kind)))]
pub fn magic(kind: Kind) -> [u8; 8] {
    match kind {
        Kind::Queue => [83, 76, 77, 84, 88, 78, 48, 49],
        Kind::Catalog => [83, 84, 82, 71, 67, 48, 48, 49],
    }
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result@ == end_view(kind)))]
pub fn end_marker(kind: Kind) -> [u8; 8] {
    match kind {
        Kind::Queue => [83, 76, 77, 84, 69, 78, 68, 49],
        Kind::Catalog => [83, 84, 82, 71, 69, 78, 68, 49],
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == range_matches(data@, start as int, expected@),
))]
pub fn matches_at(data: &[u8], start: usize, expected: &[u8]) -> bool {
    if start > data.len() || expected.len() > data.len() - start {
        return false;
    }
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant start <= data.len(), expected.len() <= data.len() - start,
            i <= expected.len(),
            forall|j: int| 0 <= j < i ==> data[start as int + j] == expected[j],
        decreases expected.len() - i,
    ))]
    while i < expected.len() {
        if data[start + i] != expected[i] {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires data.len() >= 8, start <= data.len() - 8,
    ensures result == le64(data@, start as int),
))]
pub fn decode_u64(data: &[u8], start: usize) -> u64 {
    (data[start] as u64)
        | ((data[start + 1] as u64) << 8)
        | ((data[start + 2] as u64) << 16)
        | ((data[start + 3] as u64) << 24)
        | ((data[start + 4] as u64) << 32)
        | ((data[start + 5] as u64) << 40)
        | ((data[start + 6] as u64) << 48)
        | ((data[start + 7] as u64) << 56)
}

#[cfg_attr(straw_verus, verus_spec(result => ensures le64(result@, 0) == value))]
pub fn encode_u64(value: u64) -> [u8; 8] {
    let result = [
        value as u8,
        (value >> 8) as u8,
        (value >> 16) as u8,
        (value >> 24) as u8,
        (value >> 32) as u8,
        (value >> 40) as u8,
        (value >> 48) as u8,
        (value >> 56) as u8,
    ];
    #[cfg(straw_verus)]
    proof! {
        assert(((value as u8 as u64)
            | (((value >> 8) as u8 as u64) << 8)
            | (((value >> 16) as u8 as u64) << 16)
            | (((value >> 24) as u8 as u64) << 24)
            | (((value >> 32) as u8 as u64) << 32)
            | (((value >> 40) as u8 as u64) << 40)
            | (((value >> 48) as u8 as u64) << 48)
            | (((value >> 56) as u8 as u64) << 56)) == value) by(bit_vector);
    }
    result
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result@.len() == 24, range_matches(result@, 0, magic_view(kind)),
        le64(result@, 8) == sequence, le64(result@, 16) == length,
))]
pub fn prefix(kind: Kind, sequence: u64, length: u64) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(&magic(kind));
    result.extend_from_slice(&encode_u64(sequence));
    result.extend_from_slice(&encode_u64(length));
    result
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires prefix.len() == 24, digest.len() == 32,
    ensures result@ == prefix@ + digest@,
))]
pub fn header(prefix: &[u8], digest: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(prefix);
    result.extend_from_slice(digest);
    result
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires digest.len() == 32,
    ensures result@ == digest@ + end_view(kind),
))]
pub fn trailer(kind: Kind, digest: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(digest);
    result.extend_from_slice(&end_marker(kind));
    result
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (partial.len() < 56 &&
        forall|j: int| 0 <= j < partial.len() && j < 8 ==> partial[j] == magic_view(kind)[j]),
))]
pub fn partial_header_valid(kind: Kind, partial: &[u8]) -> bool {
    if partial.len() >= 56 {
        return false;
    }
    let expected = magic(kind);
    let n = if partial.len() < 8 { partial.len() } else { 8 };
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= n, n <= 8, n <= partial.len(), partial.len() < 56,
            n == if partial.len() < 8 { partial.len() } else { 8 },
            expected@ == magic_view(kind),
            forall|j: int| 0 <= j < i ==> partial[j] == expected[j],
        decreases n - i,
    ))]
    while i < n {
        if partial[i] != expected[i] {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_some() == header_valid(kind, data@, sequence, digest@),
        result.is_some() ==> result.unwrap() == le64(data@, 16),
))]
pub fn parse_header(kind: Kind, data: &[u8], sequence: u64, digest: &[u8]) -> Option<u64> {
    if data.len() != 56 || digest.len() != 32 {
        return None;
    }
    if !matches_at(data, 0, &magic(kind)) || !matches_at(data, 24, digest) {
        return None;
    }
    if decode_u64(data, 8) != sequence {
        return None;
    }
    Some(decode_u64(data, 16))
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (data.len() == 40 && digest.len() == 32
        && range_matches(data@, 0, digest@) && range_matches(data@, 32, end_view(kind))),
))]
pub fn trailer_valid(kind: Kind, data: &[u8], digest: &[u8]) -> bool {
    data.len() == 40
        && digest.len() == 32
        && matches_at(data, 0, digest)
        && matches_at(data, 32, &end_marker(kind))
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn magic_view(kind: Kind) -> Seq<u8> {
        match kind {
            Kind::Queue => seq![83u8, 76, 77, 84, 88, 78, 48, 49],
            Kind::Catalog => seq![83u8, 84, 82, 71, 67, 48, 48, 49],
        }
    }
    pub open spec fn end_view(kind: Kind) -> Seq<u8> {
        match kind {
            Kind::Queue => seq![83u8, 76, 77, 84, 69, 78, 68, 49],
            Kind::Catalog => seq![83u8, 84, 82, 71, 69, 78, 68, 49],
        }
    }
    pub open spec fn range_matches(data: Seq<u8>, start: int, expected: Seq<u8>) -> bool {
        0 <= start && start + expected.len() <= data.len()
        && forall|j: int| 0 <= j < expected.len() ==> data[start + j] == expected[j]
    }
    pub open spec fn le64(data: Seq<u8>, start: int) -> u64 {
        (data[start] as u64) | ((data[start+1] as u64) << 8)
        | ((data[start+2] as u64) << 16) | ((data[start+3] as u64) << 24)
        | ((data[start+4] as u64) << 32) | ((data[start+5] as u64) << 40)
        | ((data[start+6] as u64) << 48) | ((data[start+7] as u64) << 56)
    }
    pub open spec fn header_valid(kind: Kind, data: Seq<u8>, sequence: u64, digest: Seq<u8>) -> bool {
        data.len() == 56 && digest.len() == 32
        && range_matches(data, 0, magic_view(kind))
        && range_matches(data, 24, digest) && le64(data, 8) == sequence
    }
    pub proof fn encoded_header_is_accepted(kind: Kind, prefix: Seq<u8>, digest: Seq<u8>, sequence: u64, length: u64)
        requires prefix.len() == 24, digest.len() == 32,
            range_matches(prefix, 0, magic_view(kind)),
            le64(prefix, 8) == sequence, le64(prefix, 16) == length,
        ensures header_valid(kind, prefix + digest, sequence, digest),
            le64(prefix + digest, 16) == length,
    {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn encoding_preserves_existing_queue_and_catalog_wire_format() {
        for kind in [Kind::Queue, Kind::Catalog] {
            for value in [0, 1, 255, 256, 1 << 32, u64::MAX - 1, u64::MAX] {
                let sequence = value;
                let length = u64::MAX - value;
                let prefix = prefix(kind, sequence, length);
                let expected_magic: &[u8] = match kind {
                    Kind::Queue => b"SLMTXN01",
                    Kind::Catalog => b"STRGC001",
                };
                let expected_end: &[u8] = match kind {
                    Kind::Queue => b"SLMTEND1",
                    Kind::Catalog => b"STRGEND1",
                };
                let mut old_prefix = expected_magic.to_vec();
                old_prefix.extend_from_slice(&sequence.to_le_bytes());
                old_prefix.extend_from_slice(&length.to_le_bytes());
                assert_eq!(prefix, old_prefix);
                let digest = Sha256::digest(&prefix);
                let header = header(&prefix, &digest);
                assert_eq!(header, [old_prefix.as_slice(), digest.as_slice()].concat());
                assert_eq!(parse_header(kind, &header, sequence, &digest), Some(length));
                assert_eq!(parse_header(kind, &header, sequence ^ 1, &digest), None);
                let trailer = trailer(kind, &digest);
                assert_eq!(trailer, [digest.as_slice(), expected_end].concat());
                assert!(trailer_valid(kind, &trailer, &digest));
            }
        }
    }

    #[test]
    fn every_corrupted_header_or_trailer_byte_is_rejected() {
        for kind in [Kind::Queue, Kind::Catalog] {
            let prefix = prefix(kind, 7, 4096);
            let header = header(&prefix, &Sha256::digest(&prefix));
            for i in 0..header.len() {
                let mut damaged = header.clone();
                damaged[i] ^= 1;
                let digest = Sha256::digest(&damaged[..24]);
                assert_eq!(parse_header(kind, &damaged, 7, &digest), None);
            }
            let digest = Sha256::digest(b"header and payload");
            let trailer = trailer(kind, &digest);
            for i in 0..trailer.len() {
                let mut damaged = trailer.clone();
                damaged[i] ^= 1;
                assert!(!trailer_valid(kind, &damaged, &digest));
            }
            // Correct checksums must not authorize a different log type.
            let other = match kind {
                Kind::Queue => Kind::Catalog,
                Kind::Catalog => Kind::Queue,
            };
            assert_eq!(
                parse_header(other, &header, 7, &Sha256::digest(&prefix)),
                None
            );
            assert!(!trailer_valid(other, &trailer, &digest));
        }
    }

    #[test]
    fn short_headers_require_the_available_magic_prefix() {
        for kind in [Kind::Queue, Kind::Catalog] {
            let prefix = prefix(kind, 0, 123);
            let header = header(&prefix, &Sha256::digest(&prefix));
            for n in 0..56 {
                assert!(partial_header_valid(kind, &header[..n]));
                assert_eq!(parse_header(kind, &header[..n], 0, &[0; 32]), None);
                for i in 0..n.min(8) {
                    let mut damaged = header[..n].to_vec();
                    damaged[i] ^= 1;
                    assert!(!partial_header_valid(kind, &damaged));
                }
            }
            assert!(!partial_header_valid(kind, &header));
            assert_eq!(parse_header(kind, &header, 0, &[]), None);
        }
    }
}
