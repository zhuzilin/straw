//! One real large publication and bounded-memory, authenticated range reads.
//! Run with `write ROOT BYTES` or `read ROOT` (descriptor JSON on stdin).
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, sync::Arc, time::Instant};
use straw::{
    Result,
    payload::{InputRecord, Payload},
    store::{Config, Reader, Store},
};

const PATTERN: usize = 8 * 1024 * 1024;
const CHUNK: usize = 256 * 1024 * 1024;

struct Repeated {
    pattern: Vec<u8>,
    length: usize,
}
impl Payload for Repeated {
    fn len(&self) -> usize {
        self.length
    }
    fn chunks(&self, size: usize, visit: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> {
        let mut offset = 0;
        while offset < self.length {
            let begin = offset % self.pattern.len();
            let n = size
                .min(self.pattern.len() - begin)
                .min(self.length - offset);
            visit(&self.pattern[begin..begin + n])?;
            offset += n;
        }
        Ok(())
    }
}
fn chunk_hash(pattern: &[u8], length: usize) -> String {
    let mut h = Sha256::new();
    for offset in (0..length).step_by(pattern.len()) {
        h.update(&pattern[..pattern.len().min(length - offset)]);
    }
    format!("{:x}", h.finalize())
}
fn rss() -> u64 {
    fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmHWM:")
                .and_then(|v| v.split_whitespace().next()?.parse().ok())
        })
        .unwrap_or(0)
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 3 {
        return straw::fail("ValueError", "Expected write ROOT BYTES or read ROOT");
    }
    let config = Config::new(
        &args[2],
        "large-probe",
        vec!["tensor.v1".into()],
        100_000_000_000,
        8 * 1024 * 1024,
        110_000_000_000,
    )?;
    if args[1] == "write" {
        let length: usize = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        if length == 0 || length > 100_000_000_000 {
            return straw::fail("ValueError", "Payload must be 1..100000000000 bytes");
        }
        let mut pattern = vec![0; PATTERN];
        fs::File::open("/dev/urandom")?.read_exact(&mut pattern)?;
        let full_hash = chunk_hash(&pattern, CHUNK);
        let hashes: Vec<_> = (0..length)
            .step_by(CHUNK)
            .map(|offset| {
                let len = CHUNK.min(length - offset);
                if len == CHUNK {
                    full_hash.clone()
                } else {
                    chunk_hash(&pattern, len)
                }
            })
            .collect();
        let record = InputRecord {
            metadata: json!({"record_id":"large", "codec":"tensor.v1", "tokens":0,
                "metadata":{"shape":[length], "dtype":"uint8", "chunk_bytes":CHUNK, "chunks":hashes}}),
            payload: Arc::new(Repeated { pattern, length }),
        };
        let mut store = Store::new(config.clone());
        let start = Instant::now();
        let result = store.publish_inputs(
            &json!([{"records":[0],"dependencies":[]}]),
            vec![record],
            "large",
            &mut |_| Ok(()),
        )?;
        let seconds = start.elapsed().as_secs_f64();
        let members = config.validate(&result[0], None, None)?;
        store.close()?;
        println!(
            "{}",
            json!({"phase":"write", "bytes":length, "seconds":seconds, "mib_s":length as f64 / 1048576.0 / seconds,
            "rss_kib":rss(), "reference":members[0], "chunk_bytes":CHUNK, "passed":true,
            "payload_policy":"Random 8 MiB pattern repeated; pattern generation and chunk metadata preparation excluded; envelope hashing, framing, write and fsync included"})
        );
    } else if args[1] == "read" {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        let descriptor: Value = serde_json::from_str(&input)?;
        let length = descriptor["bytes"].as_u64().unwrap();
        let mut reader = Reader::new(config);
        let start = Instant::now();
        let mut checked = 0;
        let mut returned = 0;
        let mut latencies = vec![];
        for offset in (0..length).step_by(CHUNK) {
            let end = length.min(offset + CHUNK as u64);
            let before = Instant::now();
            let (_, payload, n) =
                reader.read_tensor_range(&descriptor["reference"], offset, end)?;
            assert_eq!(payload.len() as u64, end - offset);
            checked += n;
            returned += payload.len() as u64;
            latencies.push(before.elapsed().as_secs_f64());
        }
        let seconds = start.elapsed().as_secs_f64();
        assert_eq!(checked, length);
        assert_eq!(returned, length);
        println!(
            "{}",
            json!({"phase":"read", "bytes":returned, "checked_bytes":checked, "seconds":seconds,
            "mib_s":returned as f64 / 1048576.0 / seconds,"rss_kib":rss(),"chunk_seconds":latencies,"passed":true})
        );
    } else {
        return straw::fail("ValueError", "Unknown operation");
    }
    Ok(())
}
