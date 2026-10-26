//! A bounded-workload native probe for preparation, readers, handoff and GC.
//! Writer prints one descriptor, then waits for `continue` after remote readers.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, Read, Write},
    sync::Arc,
    time::Instant,
};
use straw::{
    Result,
    payload::{InputRecord, Payload},
    store::{Config, Reader, Store},
};
const PATTERN: usize = 8 * 1024 * 1024;
const DEFAULT_CHUNK: usize = 256 * 1024 * 1024;
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
            let count = size
                .min(self.pattern.len() - begin)
                .min(self.length - offset);
            visit(&self.pattern[begin..begin + count])?;
            offset += count;
        }
        Ok(())
    }
}
fn checksum(pattern: &[u8], length: usize) -> String {
    let mut hash = Sha256::new();
    for offset in (0..length).step_by(pattern.len()) {
        hash.update(&pattern[..pattern.len().min(length - offset)]);
    }
    format!("{:x}", hash.finalize())
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
fn config(root: &str) -> Result<Config> {
    let mut config = Config::new(
        root,
        "workflow-probe",
        vec!["tensor.v1".into()],
        100_000_000_000,
        8 * 1024 * 1024,
        0,
    )?;
    config.online_gc = true;
    Ok(config)
}
fn emit(value: &Value) -> Result<()> {
    let mut output = std::io::stdout().lock();
    writeln!(output, "{value}")?;
    output.flush()?;
    Ok(())
}
fn write(root: &str, length: usize, chunk: usize) -> Result<()> {
    if length == 0
        || length > 100_000_000_000
        || chunk == 0
        || chunk > DEFAULT_CHUNK
        || !chunk.is_multiple_of(PATTERN)
    {
        return straw::fail(
            "ValueError",
            "Expected 1..100000000000 bytes and 8 MiB-multiple chunk <=256 MiB",
        );
    }
    let cfg = config(root)?;
    if cfg.root.exists() {
        return straw::fail("ValueError", "Probe requires a fresh private pool");
    }
    let mut pattern = vec![0; length.min(PATTERN)];
    fs::File::open("/dev/urandom")?.read_exact(&mut pattern)?;
    let full_hash = checksum(&pattern, chunk);
    let hashes: Vec<_> = (0..length)
        .step_by(chunk)
        .map(|offset| {
            let count = chunk.min(length - offset);
            if count == chunk {
                full_hash.clone()
            } else {
                checksum(&pattern, count)
            }
        })
        .collect();
    let record = InputRecord {
        metadata: json!({"record_id":"large","codec":"tensor.v1","tokens":0,
            "metadata":{"shape":[length],"dtype":"uint8","chunk_bytes":chunk,"chunks":hashes}}),
        payload: Arc::new(Repeated { pattern, length }),
    };
    let mut source = Store::new(cfg.clone());
    let start = Instant::now();
    let refs = source.write_inputs(&[record], "source", &mut |_| Ok(()))?;
    let publication = start.elapsed().as_secs_f64();
    let path = cfg.path(refs[0]["segment"]["path"].as_str().unwrap())?;
    let file_bytes = path.metadata()?.len();
    let mut target = Store::new(cfg.clone());
    let start = Instant::now();
    let mut fired = false;
    let error = target
        .record_set(&refs, &[], "destination", &mut |phase| {
            if phase == "before_footer" && !fired {
                fired = true;
                return straw::fail("StorageUnavailable", "injected incomplete destination");
            }
            Ok(())
        })
        .unwrap_err();
    let preparation = start.elapsed().as_secs_f64();
    assert!(fired && error.kind == "StorageUnavailable");
    cfg.catalog(|c| c.consume(&cfg, &refs, &mut |_| Ok(())))?;
    source.close()?;
    let before = cfg.catalog(|c| c.collect(&cfg, &mut |_| Ok(())))?.unwrap();
    assert_eq!(before["owners"], 1);
    assert_eq!(before["open_packs"], 0);
    assert_eq!(before["reclaimed_files"], 0);
    assert!(path.exists());
    let ready = json!({"phase":"ready","payload_bytes":length,"file_bytes":file_bytes,
        "reference":refs[0],"chunk_bytes":chunk,"chunk_count":length.div_ceil(chunk),
        "publication_seconds":publication,"publication_mib_s":length as f64/1048576.0/publication,
        "preparation_failure_seconds":preparation,"before_readers_gc":before,"writer_rss_kib":rss()});
    emit(&ready)?;
    let mut command = String::new();
    std::io::stdin().lock().read_line(&mut command)?;
    if command.trim() != "continue" {
        return straw::fail("ValueError", "Reader supervisor did not confirm completion");
    }
    let start = Instant::now();
    let result = target.record_set(&refs, &[], "destination", &mut |_| Ok(()))?;
    let retry = start.elapsed().as_secs_f64();
    target.close()?;
    let mut abandoned = Store::new(cfg.clone());
    let duplicate = [refs[0].clone(), refs[0].clone()];
    assert_eq!(
        abandoned
            .record_set(&duplicate, &[], "abandoned", &mut |_| Ok(()))
            .unwrap_err()
            .kind,
        "InvalidReference"
    );
    let start = Instant::now();
    abandoned.close()?;
    let close = start.elapsed().as_secs_f64();
    let protected = cfg.catalog(|c| c.collect(&cfg, &mut |_| Ok(())))?.unwrap();
    assert_eq!(protected["owners"], 1);
    assert_eq!(protected["reclaimed_files"], 0);
    assert!(path.exists());
    let start = Instant::now();
    let mut lost = false;
    let error = cfg
        .catalog(|c| {
            c.consume(&cfg, std::slice::from_ref(&result), &mut |phase| {
                if phase == "after_storage_catalog_sync" && !lost {
                    lost = true;
                    return straw::fail("StorageUnavailable", "injected consume response loss");
                }
                Ok(())
            })
        })
        .unwrap_err();
    assert!(lost && error.kind == "StorageUnavailable");
    let lost_release = start.elapsed().as_secs_f64();
    let peer = config(root)?;
    let start = Instant::now();
    let gc = peer
        .catalog(|c| c.collect(&peer, &mut |_| Ok(())))?
        .unwrap();
    let collection = start.elapsed().as_secs_f64();
    assert_eq!(gc["owners"], 0);
    assert_eq!(gc["open_packs"], 0);
    assert_eq!(gc["reclaimed_files"], 2);
    assert!(!path.exists());
    let recovered = config(root)?;
    let mut reads = 0;
    let mut appends = 0;
    let start = Instant::now();
    recovered.catalog(|c| {
        c.consume(&recovered, std::slice::from_ref(&result), &mut |phase| {
            reads += usize::from(phase == "before_staging_manifest_read");
            appends += usize::from(phase == "before_storage_catalog_write");
            Ok(())
        })
    })?;
    let release_retry = start.elapsed().as_secs_f64();
    assert_eq!((reads, appends), (0, 0));
    emit(
        &json!({"phase":"finished","passed":true,"payload_bytes":length,"file_bytes":file_bytes,
        "publication_seconds":publication,"publication_mib_s":length as f64/1048576.0/publication,
        "preparation_failure_seconds":preparation,"handoff_retry_seconds":retry,"abandoned_close_seconds":close,
        "lost_release_seconds":lost_release,"gc_seconds":collection,"post_gc_release_retry_seconds":release_retry,
        "retry_manifest_reads":reads,"retry_catalog_appends":appends,"before_readers_gc":before,
        "protected_gc":protected,"final_gc":gc,"rss_kib":rss(),"pattern_bytes":length.min(PATTERN)}),
    )
}
fn read(root: &str, shard: usize, shards: usize) -> Result<()> {
    if shards == 0 || shard >= shards {
        return straw::fail("ValueError", "Invalid reader shard");
    }
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let descriptor: Value = serde_json::from_str(&text)?;
    let length = descriptor["payload_bytes"].as_u64().unwrap();
    let chunk = descriptor["chunk_bytes"].as_u64().unwrap();
    let mut reader = Reader::new(config(root)?);
    let start = Instant::now();
    let mut returned = 0u64;
    let mut checked = 0u64;
    let mut chunks = vec![];
    for (i, offset) in (0..length).step_by(chunk as usize).enumerate() {
        if i % shards != shard {
            continue;
        }
        let end = length.min(offset + chunk);
        let before = Instant::now();
        let (_, data, authenticated) =
            reader.read_tensor_range(&descriptor["reference"], offset, end)?;
        assert_eq!(data.len() as u64, end - offset);
        assert_eq!(authenticated, end - offset);
        returned += data.len() as u64;
        checked += authenticated;
        chunks.push(json!({"index":i,"offset":offset,"bytes":authenticated,"seconds":before.elapsed().as_secs_f64()}));
    }
    let seconds = start.elapsed().as_secs_f64();
    emit(
        &json!({"passed":true,"shard":shard,"shards":shards,"returned_bytes":returned,
        "checked_bytes":checked,"seconds":seconds,"mib_s":returned as f64/1048576.0/seconds,
        "chunks":chunks,"rss_kib":rss()}),
    )
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 {
        return straw::fail(
            "ValueError",
            "Expected write ROOT BYTES CHUNK or read ROOT SHARD SHARDS",
        );
    }
    let left = args[3].parse().unwrap_or(0);
    let right = args[4].parse().unwrap_or(0);
    match args[1].as_str() {
        "write" => write(&args[2], left, right),
        "read" => read(&args[2], left, right),
        _ => straw::fail("ValueError", "Unknown workflow operation"),
    }
}
