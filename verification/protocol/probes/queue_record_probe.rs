//! Exercise queue recovery and durable reader ownership over one large record.
//! Only the supervisor may acknowledge a shard after its reader has succeeded.
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
    coordinator::Coordinator,
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
        "queue-record-probe",
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
fn queue(root: &str, recover: bool, fault: &mut straw::Fault<'_>) -> Result<Coordinator> {
    Coordinator::open(
        config(root)?,
        &json!({"exclusive_owner":"probe owner is exclusive","recover":recover}),
        fault,
    )
}
fn call(queue: &mut Coordinator, method: &str, args: Value) -> Result<Value> {
    queue.call(method, &args, 0.0, &mut |_| Ok(()))
}
fn task(id: &str, reference: &Value) -> Value {
    json!({"task_id":id,"input_ref":reference,"metadata":{},"allow_empty":false,
        "max_attempts":3,"estimated_records":1,"estimated_bytes":0,"estimated_tokens":0,
        "control":false,"priority":0,"scheduling_key":0})
}
fn write(root: &str, length: usize, chunk: usize, history: usize) -> Result<()> {
    if length == 0
        || length > 100_000_000_000
        || chunk == 0
        || chunk > DEFAULT_CHUNK
        || !chunk.is_multiple_of(PATTERN)
        || history > 10_000
    {
        return straw::fail("ValueError", "Invalid probe size/chunk/history");
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
    let raw = source.write_inputs(&[record], "source", &mut |_| Ok(()))?;
    let publication_seconds = start.elapsed().as_secs_f64();
    let data_path = cfg.path(raw[0]["segment"]["path"].as_str().unwrap())?;
    let data_bytes = data_path.metadata()?.len();
    let start = Instant::now();
    let reference = source.record_set(&raw, &[], "manifest", &mut |_| Ok(()))?;
    let manifest_seconds = start.elapsed().as_secs_f64();
    source.close()?;
    let mut q = queue(root, false, &mut |_| Ok(()))?;
    let tasks: Vec<_> = (0..4)
        .map(|i| task(&format!("reader-{i}"), &reference))
        .collect();
    let submission = json!({"request_id":"submit","tasks":tasks});
    let start = Instant::now();
    let mut cut = false;
    let error = q
        .call("submit_tasks", &submission, 0.0, &mut |at| {
            if at == "before_staging_consume" {
                cut = true;
                return straw::fail("StorageUnavailable", "accepted cleanup interrupted");
            }
            Ok(())
        })
        .unwrap_err();
    assert!(cut && error.kind == "StorageUnavailable");
    let interrupted_seconds = start.elapsed().as_secs_f64();
    q.close();
    drop(q);
    let start = Instant::now();
    let mut q = queue(root, true, &mut |_| Ok(()))?;
    let pending_recovery_seconds = start.elapsed().as_secs_f64();
    assert_eq!(
        call(&mut q, "submit_tasks", submission)?,
        json!(["reader-0", "reader-1", "reader-2", "reader-3"])
    );
    // Real accepted/cancelled history, not offline synthesized catalog frames.
    // Main tasks protect the same immutable root while history grows.
    let start = Instant::now();
    for i in 0..history {
        let id = format!("history-{i}");
        call(
            &mut q,
            "submit_tasks",
            json!({"request_id":id,"tasks":[task(&id,&reference)]}),
        )?;
        call(
            &mut q,
            "cancel_task",
            json!({"request_id":format!("cancel-{id}"),"task_id":id}),
        )?;
    }
    let history_seconds = start.elapsed().as_secs_f64();
    let mut leases = vec![];
    for i in 0..4 {
        let id = format!("reader-{i}");
        let acquired = call(
            &mut q,
            "acquire",
            json!({"worker_id":id,"task_ids":[id],"max_tasks":1}),
        )?;
        assert_eq!(acquired["assignments"].as_array().unwrap().len(), 1);
        leases.push(acquired["assignments"][0]["lease"].clone());
        call(
            &mut q,
            "cancel_task",
            json!({"request_id":format!("cancel-reader-{i}"),"task_id":id}),
        )?;
    }
    q.close();
    drop(q);
    let start = Instant::now();
    let (mut reads, mut appends) = (0, 0);
    let mut q = queue(root, true, &mut |at| {
        reads += usize::from(at == "before_staging_manifest_read");
        appends += usize::from(at == "before_storage_catalog_write");
        Ok(())
    })?;
    let history_recovery_seconds = start.elapsed().as_secs_f64();
    assert_eq!((reads, appends), (0, 0));
    assert_eq!(
        call(&mut q, "state", json!({}))?["readers"]
            .as_object()
            .unwrap()
            .len(),
        4
    );
    let protected = call(&mut q, "collect_garbage", json!({}))?;
    assert_eq!(protected["owners"], 1);
    assert_eq!(protected["open_packs"], 0);
    assert_eq!(protected["reclaimed_files"], 0);
    let wal_bytes = q.journal().path().metadata()?.len();
    let catalog_bytes = cfg.root.join("storage.log").metadata()?.len();
    emit(
        &json!({"phase":"ready","payload_bytes":length,"file_bytes":data_bytes,
        "reference":raw[0],"root_reference":reference,"chunk_bytes":chunk,"chunk_count":length.div_ceil(chunk),
        "publication_seconds":publication_seconds,"publication_mib_s":length as f64/1048576.0/publication_seconds,
        "manifest_seconds":manifest_seconds,"interrupted_seconds":interrupted_seconds,
        "pending_recovery_seconds":pending_recovery_seconds,"history":history,"history_seconds":history_seconds,
        "history_recovery_seconds":history_recovery_seconds,"recovery_manifest_reads":reads,
        "recovery_catalog_appends":appends,"wal_bytes":wal_bytes,"catalog_bytes":catalog_bytes,
        "protected_gc":protected,"writer_rss_kib":rss()}),
    )?;
    let mut completed = std::collections::HashSet::new();
    let input = std::io::stdin();
    let mut input = input.lock();
    let mut reclamation = Value::Null;
    for _ in 0..4 {
        let mut line = String::new();
        input.read_line(&mut line)?;
        let shard = line
            .trim()
            .strip_prefix("reader ")
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&i| i < 4)
            .ok_or_else(|| straw::Error::new("ValueError", "Expected successful reader index"))?;
        assert!(completed.insert(shard));
        let start = Instant::now();
        call(
            &mut q,
            "release_task_reads",
            json!({"leases":[leases[shard]]}),
        )?;
        let gc = call(&mut q, "collect_garbage", json!({}))?;
        let remaining = call(&mut q, "state", json!({}))?["readers"]
            .as_object()
            .unwrap()
            .len();
        assert_eq!(remaining, 4 - completed.len());
        if remaining > 0 {
            assert_eq!(gc["owners"], 1);
            assert_eq!(gc["reclaimed_files"], 0);
            assert!(data_path.exists());
        } else {
            assert_eq!(gc["owners"], 0);
            assert_eq!(gc["open_packs"], 0);
            assert_eq!(gc["reclaimed_files"], 2);
            assert!(!data_path.exists());
            reclamation = gc.clone();
        }
        emit(
            &json!({"phase":"reader_ack","shard":shard,"remaining":remaining,"gc":gc,"seconds":start.elapsed().as_secs_f64()}),
        )?;
    }
    q.close();
    drop(q);
    let start = Instant::now();
    let (mut reads, mut appends) = (0, 0);
    let mut q = queue(root, true, &mut |at| {
        reads += usize::from(at == "before_staging_manifest_read");
        appends += usize::from(at == "before_storage_catalog_write");
        Ok(())
    })?;
    let post_gc_recovery_seconds = start.elapsed().as_secs_f64();
    assert_eq!((reads, appends), (0, 0));
    let final_gc = call(&mut q, "collect_garbage", json!({}))?;
    assert_eq!(final_gc["owners"], 0);
    assert_eq!(final_gc["reclaimed_files"], 0);
    q.close();
    emit(
        &json!({"phase":"finished","passed":true,"payload_bytes":length,"history":history,
        "reclaimed":reclamation,"final_gc":final_gc,"post_gc_recovery_seconds":post_gc_recovery_seconds,
        "retry_manifest_reads":reads,"retry_catalog_appends":appends,"rss_kib":rss()}),
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
fn recover(root: &str, samples: usize) -> Result<()> {
    if samples == 0 || samples > 100 {
        return straw::fail("ValueError", "Expected 1..100 recovery samples");
    }
    let cfg = config(root)?;
    let catalog_bytes = cfg.root.join("storage.log").metadata()?.len();
    let mut rows = vec![];
    for sample in 0..samples {
        let before = cfg.root.join("control/journal.log").metadata()?.len();
        let (mut reads, mut appends) = (0, 0);
        let start = Instant::now();
        let mut q = queue(root, true, &mut |at| {
            reads += usize::from(at == "before_staging_manifest_read");
            appends += usize::from(at == "before_storage_catalog_write");
            Ok(())
        })?;
        let seconds = start.elapsed().as_secs_f64();
        assert_eq!((reads, appends), (0, 0));
        assert!(q.state()["readers"].as_object().unwrap().is_empty());
        assert!(
            q.state()["tasks"]
                .as_object()
                .unwrap()
                .values()
                .all(|t| t["state"] == "cancelled")
        );
        let sequence = q.journal().sequence;
        q.close();
        drop(q);
        rows.push(json!({"sample":sample,"seconds":seconds,"wal_bytes_before":before,
            "wal_bytes_after":cfg.root.join("control/journal.log").metadata()?.len(),"sequence_after":sequence,
            "manifest_reads":reads,"catalog_appends":appends,"rss_kib":rss()}));
    }
    assert_eq!(
        cfg.root.join("storage.log").metadata()?.len(),
        catalog_bytes
    );
    emit(&json!({"passed":true,"catalog_bytes":catalog_bytes,"samples":rows}))
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("write") if args.len() == 6 => write(
            &args[2],
            args[3].parse().unwrap_or(0),
            args[4].parse().unwrap_or(0),
            args[5].parse().unwrap_or(usize::MAX),
        ),
        Some("recover") if args.len() == 4 => recover(&args[2], args[3].parse().unwrap_or(0)),
        Some("read") if args.len() == 5 => read(
            &args[2],
            args[3].parse().unwrap_or(usize::MAX),
            args[4].parse().unwrap_or(0),
        ),
        _ => straw::fail(
            "ValueError",
            "Expected write ROOT BYTES CHUNK HISTORY, read ROOT SHARD SHARDS or recover ROOT SAMPLES",
        ),
    }
}
