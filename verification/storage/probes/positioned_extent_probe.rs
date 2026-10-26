//! Position-dependent payloads, independent of a repeated source pattern.
//! This measurement harness is not part of the Verus-proved production core.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, io::Read, sync::Arc, time::Instant};
use straw::{
    Result,
    payload::{InputRecord, Payload},
    store::{Config, Reader, Store},
};

const BUFFER: usize = 1024 * 1024;
const READ_WINDOW: u64 = 256 * 1024 * 1024;
const MAX_BYTES: usize = 100_000_000_000;

// SplitMix64 finalizer on the global word index. Unlike a repeated block,
// different aligned 64-bit word positions have different values. The reader
// checks returned bytes, in addition to production chunk authentication.
fn word(index: u64, seed: u64) -> u64 {
    let mut value = index.wrapping_add(seed);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}
fn fill(offset: u64, seed: u64, bytes: &mut [u8]) {
    let mut done = 0;
    while done < bytes.len() {
        let position = offset + done as u64;
        let value = word(position / 8, seed).to_le_bytes();
        let begin = (position % 8) as usize;
        let count = (8 - begin).min(bytes.len() - done);
        bytes[done..done + count].copy_from_slice(&value[begin..begin + count]);
        done += count;
    }
}
struct Positioned {
    length: usize,
    seed: u64,
}
impl Payload for Positioned {
    fn len(&self) -> usize {
        self.length
    }
    fn chunks(&self, size: usize, visit: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> {
        if size == 0 {
            return straw::fail("ValueError", "Zero payload callback size");
        }
        let mut buffer = vec![0; size.min(BUFFER).min(self.length)];
        for offset in (0..self.length).step_by(buffer.len()) {
            let count = buffer.len().min(self.length - offset);
            fill(offset as u64, self.seed, &mut buffer[..count]);
            visit(&buffer[..count])?;
        }
        Ok(())
    }
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
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn config(root: &str) -> Result<Config> {
    let mut config = Config::new(
        root,
        "positioned-probe",
        vec!["tensor.v1".into()],
        MAX_BYTES as u64,
        8 * BUFFER as u64,
        0,
    )?;
    config.online_gc = true;
    Ok(config)
}
fn write(root: &str, length: usize, chunk: usize) -> Result<()> {
    if length == 0 || length > MAX_BYTES || chunk == 0 || length.div_ceil(chunk) > 65536 {
        return straw::fail("ValueError", "Invalid workload length/chunk count");
    }
    let cfg = config(root)?;
    if cfg.root.exists() {
        return straw::fail("ValueError", "Probe requires a fresh private pool");
    }
    let mut seed = [0; 8];
    fs::File::open("/dev/urandom")?.read_exact(&mut seed)?;
    let seed = u64::from_le_bytes(seed);
    let start = Instant::now();
    let mut hashes = vec![];
    let mut buffer = vec![0; BUFFER.min(length)];
    for offset in (0..length).step_by(chunk) {
        let end = length.min(offset + chunk);
        let mut hash = Sha256::new();
        for position in (offset..end).step_by(buffer.len()) {
            let count = buffer.len().min(end - position);
            fill(position as u64, seed, &mut buffer[..count]);
            hash.update(&buffer[..count]);
        }
        hashes.push(format!("{:x}", hash.finalize()));
    }
    assert_eq!(hashes.iter().collect::<BTreeSet<_>>().len(), hashes.len());
    let preparation_seconds = start.elapsed().as_secs_f64();
    let record = InputRecord {
        metadata: json!({"record_id":"positioned","codec":"tensor.v1","tokens":0,
            "metadata":{"shape":[length],"dtype":"uint8","chunk_bytes":chunk,"chunks":hashes}}),
        payload: Arc::new(Positioned { length, seed }),
    };
    let mut store = Store::new(cfg.clone());
    let start = Instant::now();
    let refs = store.write_inputs(&[record], "source", &mut |_| Ok(()))?;
    store.close()?;
    let publication_seconds = start.elapsed().as_secs_f64();
    let path = cfg.path(refs[0]["segment"]["path"].as_str().unwrap())?;
    let protected = cfg.catalog(|c| c.collect(&cfg, &mut |_| Ok(())))?.unwrap();
    assert_eq!(protected["reclaimed_files"], 0);
    assert_eq!(protected["open_packs"], 0);
    println!(
        "{}",
        json!({"passed":true,"payload_bytes":length,"chunk_bytes":chunk,
        "seed":seed,"reference":refs[0],"hashes":hashes,"file_bytes":path.metadata()?.len(),
        "preparation_seconds":preparation_seconds,"publication_seconds":publication_seconds,
        "publication_mib_s":length as f64/1048576.0/publication_seconds,"rss_kib":rss(),
        "protected_gc":protected})
    );
    Ok(())
}
fn ranges(length: u64, chunk: u64) -> Vec<(u64, u64)> {
    let mut result = BTreeSet::new();
    for boundary in [
        0,
        8,
        BUFFER as u64,
        chunk,
        2 * chunk,
        1 << 31,
        1 << 32,
        length / 2,
        length,
    ] {
        for start in [
            boundary.saturating_sub(3),
            boundary,
            boundary.saturating_add(1),
        ] {
            if start <= length {
                for width in [0, 1, 17, 4099] {
                    result.insert((start, length.min(start + width)));
                }
            }
        }
    }
    result.into_iter().collect()
}
fn read(root: &str, descriptor: &Value, shard: usize, shards: usize) -> Result<()> {
    if shards == 0 || shard >= shards {
        return straw::fail("ValueError", "Invalid shard");
    }
    let length = descriptor["payload_bytes"].as_u64().unwrap();
    let chunk = descriptor["chunk_bytes"].as_u64().unwrap();
    let seed = descriptor["seed"].as_u64().unwrap();
    let mut reader = Reader::new(config(root)?);
    let mut expected = vec![0; BUFFER];
    // Keep requested intervals fixed while varying checksum granularity.
    // The experiment's hash-count cap is not a format or library limit.
    let full = (0..length)
        .step_by(READ_WINDOW as usize)
        .map(|start| (start, length.min(start + READ_WINDOW)));
    let requests: Vec<_> = full
        .map(|r| ("full", r))
        .chain(
            ranges(length, READ_WINDOW)
                .into_iter()
                .map(|r| ("range", r)),
        )
        .collect();
    let mut rows = vec![];
    let start = Instant::now();
    for (i, (kind, (begin, end))) in requests.into_iter().enumerate() {
        if i % shards != shard {
            continue;
        }
        let before = Instant::now();
        let (_, bytes, checked) = reader.read_tensor_range(&descriptor["reference"], begin, end)?;
        let read_seconds = before.elapsed().as_secs_f64();
        assert_eq!(bytes.len() as u64, end - begin);
        let expected_checked = if begin == end {
            0
        } else {
            length.min(((end - 1) / chunk + 1) * chunk) - begin / chunk * chunk
        };
        assert_eq!(checked, expected_checked);
        let before = Instant::now();
        for (j, data) in bytes.chunks(BUFFER).enumerate() {
            fill(
                begin + (j * BUFFER) as u64,
                seed,
                &mut expected[..data.len()],
            );
            assert_eq!(data, &expected[..data.len()], "Wrong bytes at {begin}");
        }
        let oracle_seconds = before.elapsed().as_secs_f64();
        rows.push(json!({"index":i,"kind":kind,"start":begin,"stop":end,"returned":bytes.len(),"checked":checked,
            "read_seconds":read_seconds,"oracle_seconds":oracle_seconds,
            "prefix":hex(&bytes[..bytes.len().min(32)]),"suffix":hex(&bytes[bytes.len().saturating_sub(32)..])}));
    }
    println!(
        "{}",
        json!({"passed":true,"shard":shard,"shards":shards,"seconds":start.elapsed().as_secs_f64(),"rss_kib":rss(),"reads":rows})
    );
    Ok(())
}
fn cleanup(root: &str, descriptor: &Value) -> Result<()> {
    let cfg = config(root)?;
    let path = cfg.path(descriptor["reference"]["segment"]["path"].as_str().unwrap())?;
    cfg.catalog(|c| c.consume(&cfg, &[descriptor["reference"].clone()], &mut |_| Ok(())))?;
    let gc = cfg.catalog(|c| c.collect(&cfg, &mut |_| Ok(())))?.unwrap();
    assert_eq!(gc["reclaimed_files"], 1);
    assert_eq!(gc["reclaimed_bytes"], descriptor["file_bytes"]);
    assert_eq!(gc["owners"], 0);
    assert_eq!(gc["open_packs"], 0);
    assert!(!path.exists());
    println!("{}", json!({"passed":true,"gc":gc}));
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 3 {
        return straw::fail("ValueError", "Expected write/read/cleanup ROOT arguments");
    }
    if args[1] == "write" && args.len() == 5 {
        return write(
            &args[2],
            args[3].parse().unwrap_or(0),
            args[4].parse().unwrap_or(0),
        );
    }
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let descriptor: Value = serde_json::from_str(&input)?;
    match args[1].as_str() {
        "read" if args.len() == 5 => read(
            &args[2],
            &descriptor,
            args[3].parse().unwrap_or(usize::MAX),
            args[4].parse().unwrap_or(0),
        ),
        "cleanup" if args.len() == 3 => cleanup(&args[2], &descriptor),
        _ => straw::fail("ValueError", "Unknown probe operation"),
    }
}
