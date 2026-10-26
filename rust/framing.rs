//! Queue/catalog wire framing. Digest computation and JSON remain adapters;
//! these are the actual byte checks used by both production replay paths.

#[derive(Clone, Copy)]
pub enum Kind {
    Queue,
    Catalog,
}

pub fn magic(kind: Kind) -> [u8; 8] {
    match kind {
        Kind::Queue => [83, 76, 77, 84, 88, 78, 48, 49],
        Kind::Catalog => [83, 84, 82, 71, 67, 48, 48, 49],
    }
}

pub fn end_marker(kind: Kind) -> [u8; 8] {
    match kind {
        Kind::Queue => [83, 76, 77, 84, 69, 78, 68, 49],
        Kind::Catalog => [83, 84, 82, 71, 69, 78, 68, 49],
    }
}

pub fn matches_at(data: &[u8], start: usize, expected: &[u8]) -> bool {
    if start > data.len() || expected.len() > data.len() - start {
        return false;
    }
    let mut i = 0usize;

    while i < expected.len() {
        if data[start + i] != expected[i] {
            return false;
        }
        i += 1;
    }
    true
}

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

pub fn encode_u64(value: u64) -> [u8; 8] {
    [
        value as u8,
        (value >> 8) as u8,
        (value >> 16) as u8,
        (value >> 24) as u8,
        (value >> 32) as u8,
        (value >> 40) as u8,
        (value >> 48) as u8,
        (value >> 56) as u8,
    ]
}

pub fn prefix(kind: Kind, sequence: u64, length: u64) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(&magic(kind));
    result.extend_from_slice(&encode_u64(sequence));
    result.extend_from_slice(&encode_u64(length));
    result
}

pub fn header(prefix: &[u8], digest: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(prefix);
    result.extend_from_slice(digest);
    result
}

pub fn trailer(kind: Kind, digest: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(digest);
    result.extend_from_slice(&end_marker(kind));
    result
}

pub fn partial_header_valid(kind: Kind, partial: &[u8]) -> bool {
    if partial.len() >= 56 {
        return false;
    }
    let expected = magic(kind);
    let n = if partial.len() < 8 { partial.len() } else { 8 };
    let mut i = 0usize;

    while i < n {
        if partial[i] != expected[i] {
            return false;
        }
        i += 1;
    }
    true
}

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

pub fn trailer_valid(kind: Kind, data: &[u8], digest: &[u8]) -> bool {
    data.len() == 40
        && digest.len() == 32
        && matches_at(data, 0, digest)
        && matches_at(data, 32, &end_marker(kind))
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
