//! One real standalone streamed record, authentication, ownership release and GC.
//! Workload cap: 100 decimal GB per record; not a library size limit.
use serde_json::json;
use std::{fs, io::Read, sync::Arc, time::Instant};
use straw::{
    Result,
    payload::{InputRecord, Payload},
    store::{Config, Store},
};

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
    if args.len() != 3 {
        return straw::fail("ValueError", "Expected ROOT BYTES");
    }
    let length: usize = args[2].parse().unwrap_or(0);
    if length == 0 || length > 100_000_000_000 {
        return straw::fail("ValueError", "Probe workload must be 1..100000000000 bytes");
    }
    let mut config = Config::new(
        &args[1],
        "standalone-probe",
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
    let record = InputRecord {
        metadata: json!({"record_id":"large","codec":"bytes.v1","tokens":0,"metadata":{}}),
        payload: Arc::new(Repeated { pattern, length }),
    };
    let mut store = Store::new(config.clone());
    let start = Instant::now();
    let refs = store.write_inputs(&[record], "large", &mut |_| Ok(()))?;
    let publication = start.elapsed().as_secs_f64();
    eprintln!("{}", json!({"phase":"published","seconds":publication}));
    let path = config.path(refs[0]["segment"]["path"].as_str().unwrap())?;
    let file_bytes = path.metadata()?.len();
    let before = config
        .catalog(|c| c.collect(&config, &mut |_| Ok(())))?
        .unwrap();
    assert_eq!(before["open_packs"], 0);
    assert_eq!(before["reclaimed_files"], 0);
    let start = Instant::now();
    config.inspect(&refs[0]["segment"], true)?;
    let verification = start.elapsed().as_secs_f64();
    eprintln!(
        "{}",
        json!({"phase":"authenticated","seconds":verification})
    );
    store.close()?;
    let start = Instant::now();
    config.catalog(|c| c.consume(&config, &refs, &mut |_| Ok(())))?;
    let release = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let gc = config
        .catalog(|c| c.collect(&config, &mut |_| Ok(())))?
        .unwrap();
    let collection = start.elapsed().as_secs_f64();
    assert_eq!(gc["reclaimed_files"], 1);
    assert_eq!(gc["reclaimed_bytes"], file_bytes);
    assert_eq!(gc["owners"], 0);
    assert!(!path.exists());
    println!(
        "{}",
        json!({"passed":true,"root":config.root,"payload_bytes":length,"file_bytes":file_bytes,"reference":refs[0],"before_release":before,"gc":gc,"publication_seconds":publication,"authentication_seconds":verification,"release_seconds":release,"gc_seconds":collection,"publication_mib_s":length as f64/1048576.0/publication,"authentication_mib_s":length as f64/1048576.0/verification,"rss_kib":rss(),"pattern_bytes":length.min(8*1024*1024),"scope":"One repeated random-pattern record, same-host authenticated read, no cache eviction; no p99 or physical-device throughput guarantee. GC timing is namespace deletion/receipt, not backend object reclamation completion."})
    );
    Ok(())
}
