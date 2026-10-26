//! One real record with a lost staging response, rejected interleaving and exact-file retry.
//! Workload cap: 100 decimal GB per record; not a library size limit.
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{fs, io::Read, sync::Arc, time::Instant};
use straw::{
    Result,
    payload::{InputRecord, Payload},
    store::{Config, Store},
};

struct Repeated {
    pattern: Vec<u8>,
    length: usize,
    visits: Arc<AtomicUsize>,
}
impl Payload for Repeated {
    fn len(&self) -> usize {
        self.length
    }
    fn chunks(&self, size: usize, visit: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> {
        self.visits.fetch_add(1, Ordering::Relaxed);
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
fn data_files(path: &std::path::Path, result: &mut Vec<std::path::PathBuf>) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            data_files(&entry.path(), result)?;
        } else if matches!(
            entry.path().extension().and_then(|s| s.to_str()),
            Some("sealed" | "partial" | "pack")
        ) {
            result.push(entry.path());
        }
    }
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return straw::fail("ValueError", "Expected ROOT BYTES");
    }
    let length: usize = args[2].parse().unwrap_or(0);
    if length == 0 || length > 100_000_000_000 {
        return straw::fail("ValueError", "Probe workload must be 1..100000000000 bytes");
    }
    let mut config = Config::new(
        &args[1],
        "handoff-probe",
        vec!["bytes.v1".into()],
        100_000_000_000,
        8 * 1024 * 1024,
        0,
    )?;
    config.online_gc = true;
    if config.root.exists() {
        return straw::fail("ValueError", "Probe requires a fresh private pool");
    }
    let mut pattern = vec![0; length.min(8 * 1024 * 1024)];
    fs::File::open("/dev/urandom")?.read_exact(&mut pattern)?;
    let visits = Arc::new(AtomicUsize::new(0));
    let record = InputRecord {
        metadata: json!({"record_id":"large","codec":"bytes.v1","tokens":0,"metadata":{}}),
        payload: Arc::new(Repeated {
            pattern,
            length,
            visits: visits.clone(),
        }),
    };
    let mut writer = Store::new(config.clone());
    let total = Instant::now();
    let start = Instant::now();
    let (mut armed, mut fired) = (false, false);
    let error = writer
        .write_inputs(std::slice::from_ref(&record), "large", &mut |phase| {
            if phase == "after_data_sync" {
                armed = true;
            }
            if armed && phase == "after_storage_catalog_sync" && !fired {
                fired = true;
                return straw::fail("StorageUnavailable", "injected staging response loss");
            }
            Ok(())
        })
        .unwrap_err();
    let initial = start.elapsed().as_secs_f64();
    assert!(fired && error.kind == "StorageUnavailable");
    assert_eq!(visits.load(Ordering::Relaxed), 2);
    eprintln!(
        "{}",
        json!({"phase":"staging_response_lost","seconds":initial})
    );
    let mut files = vec![];
    data_files(&config.root, &mut files)?;
    assert_eq!(files.len(), 1);
    let original = files.pop().unwrap();
    let metadata = original.metadata()?;
    let before = config
        .catalog(|c| c.collect(&config, &mut |_| Ok(())))?
        .unwrap();
    assert_eq!(before["reclaimed_files"], 0);
    assert_eq!(before["open_packs"], 1);
    let start = Instant::now();
    let error = writer
        .write_inputs(std::slice::from_ref(&record), "other", &mut |_| Ok(()))
        .unwrap_err();
    let rejected = start.elapsed().as_secs_f64();
    assert_eq!(error.kind, "IndeterminateCommit");
    assert_eq!(visits.load(Ordering::Relaxed), 2);
    assert_eq!(
        writer.seal(&mut |_| Ok(())).unwrap_err().kind,
        "IndeterminateCommit"
    );
    let mut payload_callbacks = 0;
    let start = Instant::now();
    let refs = writer.write_inputs(std::slice::from_ref(&record), "large", &mut |phase| {
        if phase == "after_payload_chunk" {
            payload_callbacks += 1;
        }
        Ok(())
    })?;
    let retry = start.elapsed().as_secs_f64();
    let total_success = total.elapsed().as_secs_f64();
    assert_eq!(visits.load(Ordering::Relaxed), 3);
    assert_eq!(payload_callbacks, 0);
    assert_eq!(
        config.path(refs[0]["segment"]["path"].as_str().unwrap())?,
        original
    );
    assert_eq!(original.metadata()?.len(), metadata.len());
    assert_eq!(original.metadata()?.modified()?, metadata.modified()?);
    eprintln!("{}", json!({"phase":"retry_completed","seconds":retry}));
    let start = Instant::now();
    config.inspect(&refs[0]["segment"], true)?;
    let verification = start.elapsed().as_secs_f64();
    eprintln!(
        "{}",
        json!({"phase":"authenticated","seconds":verification})
    );
    let start = Instant::now();
    config.catalog(|c| c.consume(&config, &refs, &mut |_| Ok(())))?;
    let release = start.elapsed().as_secs_f64();
    writer.close()?;
    let start = Instant::now();
    let gc = config
        .catalog(|c| c.collect(&config, &mut |_| Ok(())))?
        .unwrap();
    let collection = start.elapsed().as_secs_f64();
    assert_eq!(gc["reclaimed_files"], 1);
    assert_eq!(gc["owners"], 0);
    assert_eq!(gc["open_packs"], 0);
    assert!(!original.exists());
    println!(
        "{}",
        json!({"passed":true,"root":config.root,"payload_bytes":length,"file_bytes":metadata.len(),"reference":refs[0],"before_retry_gc":before,"gc":gc,"initial_failed_publication_seconds":initial,"rejected_other_seconds":rejected,"retry_seconds":retry,"total_success_seconds":total_success,"authentication_seconds":verification,"release_seconds":release,"gc_seconds":collection,"initial_write_mib_s":length as f64/1048576.0/initial,"workflow_mib_s":length as f64/1048576.0/total_success,"authentication_mib_s":length as f64/1048576.0/verification,"rss_kib":rss(),"payload_visits":visits.load(Ordering::Relaxed),"retry_payload_write_callbacks":payload_callbacks,"pattern_bytes":length.min(8*1024*1024),"scope":"One repeated random-pattern record and staging error, same-host authentication, no cache eviction. Initial timing ends in error; workflow includes diagnostics and rejection. Retry rehashes input once but emits no payload writes. No p99, physical-device bandwidth or backend-GC completion claim."})
    );
    Ok(())
}
