//! Stream a large publication, then measure real catalog reactivation/GC.
//! `prepare ROOT BYTES`; `measure ROOT SAMPLES` takes the descriptor on stdin.
use serde_json::{Value, json};
use std::{fs, io::Read, sync::Arc, time::Instant};
use straw::{
    Result,
    gc::Catalog,
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
    if args.len() != 4 {
        return straw::fail(
            "ValueError",
            "Expected prepare ROOT BYTES or measure ROOT SAMPLES",
        );
    }
    let mut config = Config::new(
        &args[2],
        "reactivation-probe",
        vec!["bytes.v1".into()],
        100_000_000_000,
        8 * 1024 * 1024,
        110_000_000_000,
    )?;
    config.online_gc = true;
    if args[1] == "prepare" {
        let length: usize = args[3].parse().unwrap_or(0);
        // Workload limit, not a library admission cap.
        if length == 0 || length > 100_000_000_000 {
            return straw::fail("ValueError", "Probe payload must be 1..100000000000 bytes");
        }
        let mut pattern = vec![0; length.min(8 * 1024 * 1024)];
        fs::File::open("/dev/urandom")?.read_exact(&mut pattern)?;
        let record = InputRecord {
            metadata: json!({"record_id":"large","codec":"bytes.v1","tokens":0,"metadata":{}}),
            payload: Arc::new(Repeated { pattern, length }),
        };
        let mut store = Store::new(config.clone());
        let started = Instant::now();
        let root = store.publish_inputs(
            &json!([{"records":[0],"dependencies":[]}]),
            vec![record],
            "large",
            &mut |_| Ok(()),
        )?[0]
            .clone();
        let seconds = started.elapsed().as_secs_f64();
        store.seal(&mut |_| Ok(()))?;
        let checkpoint = store.publish_inputs(
            &json!([{"records":[0],"dependencies":[root]}]),
            vec![InputRecord {
                metadata: json!({"record_id":"checkpoint","codec":"bytes.v1","tokens":0,"metadata":{}}),
                payload: Arc::new(Arc::<[u8]>::from(&b"saved"[..])),
            }], "checkpoint", &mut |_| Ok(()),
        )?[0].clone();
        store.close()?;
        config.catalog(|c| {
            c.retain(
                &config,
                "checkpoint",
                std::slice::from_ref(&checkpoint),
                &mut |_| Ok(()),
            )?;
            c.release(
                &config,
                &[Catalog::staged(&root)?, Catalog::staged(&checkpoint)?],
                &mut |_| Ok(()),
            )
        })?;
        println!(
            "{}",
            json!({"phase":"prepare","bytes":length,"reference":root,
            "publication_seconds":seconds,"publication_mib_s":length as f64 / 1048576.0 / seconds,
            "rss_kib":rss(),"passed":true})
        );
    } else if args[1] == "measure" || args[1] == "measure-locked" {
        let samples: usize = args[3].parse().unwrap_or(0);
        if samples == 0 || samples > 10000 {
            return straw::fail("ValueError", "Probe samples must be 1..10000");
        }
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        let descriptor: Value = serde_json::from_str(&input)?;
        let reference = descriptor["reference"].clone();
        let bytes = descriptor["bytes"].as_u64().unwrap();
        let owner = "restored".to_string();
        let mut latencies = Vec::new();
        let mut events = Vec::new();
        let marker_dir = std::env::var("STRAW_REACTIVATION_MARKER_DIR").ok();
        let pause_ms = std::env::var("STRAW_REACTIVATION_PAUSE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        for sample in 0..samples {
            let mut trace = Vec::new();
            let started = Instant::now();
            let mut fault = |phase: &str| -> Result<()> {
                // Local monotonic observations, not synchronized host clocks.
                // Hooks describe coarse boundaries; e.g. final catalog time
                // includes lock acquisition, refresh, and authorization.
                trace.push(json!({"phase":phase,"seconds":started.elapsed().as_secs_f64()}));
                if let Some(directory) = &marker_dir {
                    if phase == "before_reactivation_verify" {
                        fs::write(std::path::Path::new(&directory).join("started"), b"started")?;
                        trace.push(json!({"phase":"after_start_marker","seconds":started.elapsed().as_secs_f64()}));
                        std::thread::sleep(std::time::Duration::from_millis(pause_ms));
                        trace.push(json!({"phase":"after_pause","seconds":started.elapsed().as_secs_f64()}));
                    } else if phase == "after_reactivation_verify" {
                        fs::write(
                            std::path::Path::new(&directory).join("finished"),
                            b"finished",
                        )?;
                        trace.push(json!({"phase":"after_finish_marker","seconds":started.elapsed().as_secs_f64()}));
                    }
                }
                Ok(())
            };
            let roots = [(owner.clone(), reference.clone())];
            if args[1] == "measure-locked" {
                // Same-binary control: caller's transaction covers verification.
                config.catalog(|c| c.retain_reactivated(&config, &roots, &mut fault))?;
            } else {
                config.retain_reactivated(&roots, &mut fault)?;
            }
            let elapsed = started.elapsed().as_secs_f64();
            latencies.push(elapsed);
            trace.push(json!({"phase":"complete","seconds":elapsed}));
            events.push(trace);
            if sample + 1 < samples {
                config.catalog(|c| {
                    c.release(&config, std::slice::from_ref(&owner), &mut |_| Ok(()))
                })?;
            }
        }
        // After the source checkpoint is gone, the restored owner alone must
        // protect the payload pack. The separate checkpoint pack can be GC'd.
        config.catalog(|c| c.release(&config, &["checkpoint".to_string()], &mut |_| Ok(())))?;
        let protected_gc = config
            .catalog(|c| c.collect(&config, &mut |_| Ok(())))?
            .unwrap();
        assert_eq!(protected_gc["reclaimed_files"], 1);
        assert!(
            config
                .path(reference["manifest"]["segment"]["path"].as_str().unwrap())?
                .exists()
        );
        config.validate(&reference, None, None)?;
        config.catalog(|c| c.release(&config, &[owner], &mut |_| Ok(())))?;
        let final_gc = config
            .catalog(|c| c.collect(&config, &mut |_| Ok(())))?
            .unwrap();
        assert_eq!(final_gc["reclaimed_files"], 1);
        let total: f64 = latencies.iter().sum();
        println!(
            "{}",
            json!({"phase":"measure","bytes":bytes,"samples":samples,
            "seconds":latencies,"events":events,"verified_payload_bytes":bytes * samples as u64,
            "mib_s":bytes as f64 * samples as f64 / 1048576.0 / total,
            "rss_kib":rss(),"protected_gc":protected_gc,"final_gc":final_gc,"passed":true,
            "operation":args[1]})
        );
    } else {
        return straw::fail("ValueError", "Unknown probe mode");
    }
    Ok(())
}
