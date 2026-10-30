use serde_json::{Value, json};
use straw::{
    Result,
    coordinator::Coordinator,
    store::{Config, Record, Store},
};
fn config(root: &std::path::Path, target: u64) -> Config {
    Config::new(
        root.to_str().unwrap(),
        "test",
        vec!["bytes.v1".into(), "json.v1".into()],
        1024 * 1024,
        4 * 1024 * 1024,
        target,
    )
    .unwrap()
}
fn record(id: &str, data: &[u8], metadata: Value) -> Record {
    Record {
        metadata: json!({"record_id":id,"codec":"bytes.v1","metadata":metadata,"tokens":0}),
        payload: data.to_vec().into(),
    }
}
fn task(id: &str) -> Value {
    json!({"task_id":id,"input_ref":null,"metadata":{},"allow_empty":false,"max_attempts":3,"estimated_records":1,"estimated_bytes":0,"estimated_tokens":0,"control":false})
}

#[test]
fn journal_recovery_preserves_exact_json_numbers_and_request_bytes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let identity = json!({"run":"json-roundtrip"});
    let event = json!([{"type":"Producer", "state":{
        "small":0.00018313042101781934_f64,
        "tiny":9.901469416441159e-145_f64,
        "large":1.294108664437241e276_f64,
        "negative_zero":-0.0_f64,
        "bounds":[u64::MAX, i64::MIN, f64::MIN_POSITIVE, f64::from_bits(1)],
    }}]);
    let canonical = straw::bytes(&event)?;
    let mut journal =
        straw::journal::Journal::open(root.path(), &identity, false, false, &mut |_| Ok(()))?;
    journal.append(&event, &mut |_| Ok(()))?;
    journal.close();
    for _ in 0..3 {
        let mut replayed =
            straw::journal::Journal::open(root.path(), &identity, true, false, &mut |_| Ok(()))?;
        assert_eq!(straw::bytes(&replayed.transactions[1])?, canonical);
        replayed.close();
    }
    Ok(())
}

#[test]
fn caught_commit_apply_unwind_cannot_prune_accepted_data() -> Result<()> {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    for phase in ["before_commit_apply", "after_commit_apply"] {
        let dir = tempfile::tempdir()?;
        let mut cfg = config(dir.path(), 0);
        cfg.online_gc = true;
        let mut writer = Store::new(cfg.clone());
        let reference = writer.publish(
            &json!([{"records":[0],"dependencies":[]}]),
            vec![record("input", b"accepted input", json!({}))],
            "input",
            &mut |_| Ok(()),
        )?[0]
            .clone();
        writer.close()?;
        let mut input = task("accepted");
        input["input_ref"] = reference.clone();
        let request = json!({"request_id":"submit","tasks":[input]});
        let mut queue = Coordinator::open(
            cfg.clone(),
            &json!({"exclusive_owner":"native test"}),
            &mut |_| Ok(()),
        )?;
        let interrupted = catch_unwind(AssertUnwindSafe(|| {
            queue.call("submit_tasks", &request, 0., &mut |at| {
                assert_ne!(at, phase, "interrupted application");
                Ok(())
            })
        }));
        assert!(interrupted.is_err());
        let sequence = queue.journal().sequence;
        let collection = queue.call("collect_garbage", &json!({}), 1., &mut |_| Ok(()));
        assert!(
            matches!(collection, Err(ref e) if e.kind == "UnsafeRecovery"),
            "{phase}: caught unwind allowed GC after durable acceptance: {collection:?}"
        );
        assert_eq!(queue.journal().sequence, sequence);
        assert!(matches!(queue.usage(), Err(ref error) if error.kind == "UnsafeRecovery"));
        assert_eq!(
            cfg.read(&cfg.validate(&reference, None, None)?[0])?
                .payload
                .as_ref(),
            b"accepted input"
        );
        queue.close();
        let mut recovered = Coordinator::open(
            cfg.clone(),
            &json!({"exclusive_owner":"previous owner closed","recover":true}),
            &mut |_| Ok(()),
        )?;
        assert_eq!(
            recovered.call("submit_tasks", &request, 2., &mut |_| Ok(()))?,
            json!(["accepted"])
        );
        assert_eq!(
            recovered.call("collect_garbage", &json!({}), 3., &mut |_| Ok(()))?["reclaimed_files"],
            0
        );
        assert_eq!(
            cfg.read(&cfg.validate(&reference, None, None)?[0])?
                .payload
                .as_ref(),
            b"accepted input"
        );
        recovered.close();
    }
    Ok(())
}

fn protected_root(cfg: &Config, id: &str) -> Result<Value> {
    let mut store = Store::new(cfg.clone());
    let reference = store.publish(
        &json!([{"records":[0],"dependencies":[]}]),
        vec![record(id, b"protected", json!({}))],
        id,
        &mut |_| Ok(()),
    )?[0]
        .clone();
    store.close()?;
    cfg.catalog(|c| {
        c.retain(cfg, id, std::slice::from_ref(&reference), &mut |_| Ok(()))?;
        c.release(cfg, &[straw::gc::Catalog::staged(&reference)?], &mut |_| {
            Ok(())
        })
    })?;
    Ok(reference)
}

#[test]
fn reactivation_rechecks_all_sources_before_any_owner_append() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut cfg = config(dir.path(), 1024 * 1024);
    cfg.online_gc = true;
    let a = protected_root(&cfg, "a")?;
    let b = protected_root(&cfg, "b")?;
    let roots = vec![("restored-a".into(), a), ("restored-b".into(), b)];
    let verified = cfg.verify_reactivation(&roots, &mut |_| Ok(()))?;
    cfg.catalog(|c| c.release(&cfg, &["b".into()], &mut |_| Ok(())))?;
    let before = std::fs::read(dir.path().join("storage.log"))?;
    let error = cfg
        .catalog(|c| c.retain_verified(&cfg, &verified, &mut |_| Ok(())))
        .unwrap_err();
    assert_eq!(error.kind, "UnsafeRecovery");
    assert_eq!(std::fs::read(dir.path().join("storage.log"))?, before);
    cfg.catalog(|c| c.collect(&cfg, &mut |_| Ok(())))?;
    let before = std::fs::read(dir.path().join("storage.log"))?;
    let error = cfg
        .catalog(|c| c.retain_verified(&cfg, &verified, &mut |_| Ok(())))
        .unwrap_err();
    assert_eq!(error.kind, "InvalidReference");
    assert_eq!(std::fs::read(dir.path().join("storage.log"))?, before);
    Ok(())
}

#[test]
fn reactivation_proof_is_bound_to_pool_and_run() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut cfg = config(dir.path(), 1024 * 1024);
    cfg.online_gc = true;
    let reference = protected_root(&cfg, "source")?;
    let verified = cfg.verify_reactivation(&[("restored".into(), reference)], &mut |_| Ok(()))?;
    let other = tempfile::tempdir()?;
    for wrong_root in [false, true] {
        let mut wrong = cfg.clone();
        if wrong_root {
            wrong.root = other.path().into();
        } else {
            wrong.run_id = "other".into();
        }
        let before = std::fs::read(dir.path().join("storage.log"))?;
        let error = cfg
            .catalog(|c| c.retain_verified(&wrong, &verified, &mut |_| Ok(())))
            .unwrap_err();
        assert_eq!(error.kind, "InvalidReference");
        assert!(error.message.contains("another storage pool"));
        assert_eq!(std::fs::read(dir.path().join("storage.log"))?, before);
    }
    Ok(())
}

#[test]
fn root_that_loses_its_owner_after_snapshot_requires_verification() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut cfg = config(dir.path(), 1024 * 1024);
    cfg.online_gc = true;
    let reference = protected_root(&cfg, "checkpoint")?;
    cfg.catalog(|c| {
        c.retain(
            &cfg,
            "restored",
            std::slice::from_ref(&reference),
            &mut |_| Ok(()),
        )
    })?;
    let roots = vec![("restored".into(), reference)];
    let verified = cfg.verify_reactivation(&roots, &mut |_| Ok(()))?;
    cfg.catalog(|c| c.release(&cfg, &["restored".into()], &mut |_| Ok(())))?;
    let before = std::fs::read(dir.path().join("storage.log"))?;
    let error = cfg
        .catalog(|c| c.retain_verified(&cfg, &verified, &mut |_| Ok(())))
        .unwrap_err();
    assert_eq!(error.kind, "UnsafeRecovery");
    assert_eq!(std::fs::read(dir.path().join("storage.log"))?, before);
    cfg.retain_reactivated(&roots, &mut |_| Ok(()))?;
    cfg.catalog(|c| c.release(&cfg, &["checkpoint".into()], &mut |_| Ok(())))?;
    assert_eq!(
        cfg.catalog(|c| c.collect(&cfg, &mut |_| Ok(())))?.unwrap()["reclaimed_files"],
        0
    );
    Ok(())
}
#[test]
fn reactivation_rejects_an_existing_owner_of_a_different_root() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut cfg = config(dir.path(), 1024 * 1024);
    cfg.online_gc = true;
    let a = protected_root(&cfg, "source-a")?;
    let b = protected_root(&cfg, "source-b")?;
    cfg.catalog(|c| c.retain(&cfg, "restored", std::slice::from_ref(&a), &mut |_| Ok(())))?;
    let before = std::fs::read(dir.path().join("storage.log"))?;
    assert!(
        cfg.verify_reactivation(&[("restored".into(), b.clone())], &mut |_| Ok(()))
            .is_err()
    );
    assert_eq!(std::fs::read(dir.path().join("storage.log"))?, before);
    // Normal reuse still succeeds, with no catalog write.
    cfg.retain_reactivated(&[("restored".into(), a)], &mut |_| Ok(()))?;
    assert_eq!(std::fs::read(dir.path().join("storage.log"))?, before);
    Ok(())
}

#[test]
fn reactivation_rechecks_an_owner_created_after_authentication() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut cfg = config(dir.path(), 1024 * 1024);
    cfg.online_gc = true;
    let a = protected_root(&cfg, "source-a")?;
    let b = protected_root(&cfg, "source-b")?;
    let verified = cfg.verify_reactivation(&[("restored".into(), b)], &mut |_| Ok(()))?;
    cfg.catalog(|c| c.retain(&cfg, "restored", &[a], &mut |_| Ok(())))?;
    let before = std::fs::read(dir.path().join("storage.log"))?;
    let error = cfg
        .catalog(|c| c.retain_verified(&cfg, &verified, &mut |_| Ok(())))
        .unwrap_err();
    assert_eq!(error.kind, "UnsafeRecovery");
    assert_eq!(std::fs::read(dir.path().join("storage.log"))?, before);
    Ok(())
}

#[test]
fn shared_validation_keeps_root_identity_checks_and_clears_failed_scope() -> Result<()> {
    use std::collections::HashSet;
    use straw::store::Reader;
    let dir = tempfile::tempdir()?;
    let mut store = Store::new(config(dir.path(), 1024 * 1024));
    let leaf = store.publish(
        &json!([{"records":[0],"dependencies":[]}]),
        vec![record(
            "leaf",
            b"data",
            json!({"task_id":"owner", "attempt_id":"a"}),
        )],
        "leaf",
        &mut |_| Ok(()),
    )?[0]
        .clone();
    let roots = store.publish(
        &json!([
            {"records":[0],"dependencies":[leaf]},
            {"records":[1],"dependencies":[leaf]}
        ]),
        vec![
            record("state", b"state", json!({})),
            record("progress", b"progress", json!({})),
        ],
        "roots",
        &mut |_| Ok(()),
    )?;
    let mut reader = Reader::new(store.config.clone());
    let mut scope = HashSet::new();
    for root in &roots {
        assert_eq!(
            reader.validate_scoped(root, None, None, &mut scope)?.len(),
            1
        );
    }
    assert_eq!(scope.len(), 3);
    // The leaf was already seen as a dependency. It must still check its own
    // task/attempt when a caller subsequently validates it as a root.
    assert_eq!(
        reader
            .validate_scoped(&leaf, Some("owner"), Some("a"), &mut scope)?
            .len(),
        1
    );
    assert!(
        reader
            .validate_scoped(&leaf, Some("other"), Some("a"), &mut scope)
            .is_err()
    );
    assert!(scope.is_empty());
    assert!(
        reader
            .validate_scoped(&leaf, Some("owner"), Some("other"), &mut scope)
            .is_err()
    );
    assert!(scope.is_empty());
    Ok(())
}

#[test]
fn validation_does_not_reuse_a_previous_operations_dependency_proof() -> Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let dir = tempfile::tempdir()?;
    let mut store = Store::new(config(dir.path(), 0));
    let leaf = store.publish(
        &json!([{"records":[0],"dependencies":[]}]),
        vec![record("leaf", b"payload", json!({}))],
        "leaf",
        &mut |_| Ok(()),
    )?[0]
        .clone();
    let root = store.publish(
        &json!([{"records":[0],"dependencies":[leaf]}]),
        vec![record("root", b"root", json!({}))],
        "root",
        &mut |_| Ok(()),
    )?[0]
        .clone();
    store.config.validate(&root, None, None)?;
    let segment = &leaf["manifest"]["segment"];
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(dir.path().join(segment["path"].as_str().unwrap()))?;
    file.seek(SeekFrom::Start(segment["offset"].as_u64().unwrap()))?;
    file.write_all(&[0; 12])?;
    file.sync_all()?;
    assert!(store.config.validate(&root, None, None).is_err());
    assert!(
        store
            .publish(
                &json!([{"records":[0],"dependencies":[root]}]),
                vec![record("next", b"next", json!({}))],
                "next",
                &mut |_| Ok(()),
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn append_100_samples_one_file_and_read_old_extents() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut store = Store::new(config(dir.path(), 1024 * 1024));
    let mut refs = vec![];
    for i in 0..100 {
        refs.push(
            store.publish(
                &json!([{"records":[0],"dependencies":[]}]),
                vec![record(&format!("s{i}"), &[i; 100], json!({}))],
                &i.to_string(),
                &mut |_| Ok(()),
            )?[0]
                .clone(),
        );
    }
    assert!(
        refs.iter()
            .all(|r| r["manifest"]["segment"]["path"] == refs[0]["manifest"]["segment"]["path"])
    );
    for (i, r) in refs.iter().enumerate() {
        let records = store.config.validate(r, None, None)?;
        assert_eq!(
            store.config.read(&records[0])?.payload.as_ref(),
            &[i as u8; 100]
        );
        store.config.inspect(&r["manifest"]["segment"], true)?;
    }
    Ok(())
}
#[test]
fn bad_dependency_header_identifies_the_actual_extent() -> Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let dir = tempfile::tempdir()?;
    let mut store = Store::new(config(dir.path(), 1024 * 1024));
    let groups = json!([{"records":[0],"dependencies":[]}]);
    store.publish(
        &groups,
        vec![record("prefix", b"prefix", json!({}))],
        "prefix",
        &mut |_| Ok(()),
    )?;
    let dep = store.publish(
        &groups,
        vec![record("dep", b"dependency", json!({}))],
        "dep",
        &mut |_| Ok(()),
    )?[0]
        .clone();
    store.seal(&mut |_| Ok(()))?;
    let parent = store.publish(
        &json!([{"records":[0],"dependencies":[dep]}]),
        vec![record("parent", b"parent", json!({}))],
        "parent",
        &mut |_| Ok(()),
    )?[0]
        .clone();
    let segment = &dep["manifest"]["segment"];
    let path = segment["path"].as_str().unwrap();
    let offset = segment["offset"].as_u64().unwrap();
    assert!(offset > 0);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(dir.path().join(path))?;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(&[0; 12])?;
    file.sync_all()?;
    let error = store.config.validate(&parent, None, None).unwrap_err();
    assert_eq!(error.kind, "CorruptData");
    assert!(error.message.starts_with("Invalid segment header;"));
    assert!(error.message.contains(path));
    assert!(error.message.contains(&format!("offset={offset},")));
    assert!(
        error
            .message
            .contains("observed_header=000000000000000000000000")
    );
    assert!(
        !error
            .message
            .contains(parent["manifest"]["segment"]["path"].as_str().unwrap())
    );
    Ok(())
}
#[test]
fn completion_replays_atomically_and_old_leases_are_fenced() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = config(dir.path(), 1024 * 1024);
    let options = json!({"exclusive_owner":"test owns lifetime"});
    let mut q = Coordinator::open(cfg.clone(), &options, &mut |_| Ok(()))?;
    q.call(
        "submit_tasks",
        &json!({"request_id":"submit","tasks":[task("a"),task("b")]}),
        1.,
        &mut |_| Ok(()),
    )?;
    let acquired = q.call(
        "acquire",
        &json!({"worker_id":"w","max_tasks":2}),
        1.,
        &mut |_| Ok(()),
    )?;
    let lease = acquired["assignments"][0]["lease"].clone();
    let old = acquired["assignments"][1]["lease"].clone();
    let mut publisher = Store::new(cfg.clone());
    let result = publisher.publish(
        &json!([{"records":[0],"dependencies":[]}]),
        vec![record(
            "r",
            b"payload",
            json!({"task_id":"a","attempt_id":lease["attempt_id"]}),
        )],
        "physical",
        &mut |_| Ok(()),
    )?[0]
        .clone();
    publisher.close()?;
    let args = json!({"lease":lease,"submission_id":"result","result_ref":result,"result_digest":result["digest"]});
    let first = q.call("complete_task", &args, 2., &mut |_| Ok(()))?;
    q.close();
    let mut recovered = Coordinator::open(
        cfg,
        &json!({"exclusive_owner":"old owner stopped","recover":true}),
        &mut |_| Ok(()),
    )?;
    assert_eq!(
        recovered.call("complete_task", &args, 3., &mut |_| Ok(()))?,
        first
    );
    assert_eq!(
        recovered.call("heartbeat", &json!({"leases":[old]}), 3., &mut |_| Ok(()))?,
        json!(["StaleAttempt"])
    );
    assert_eq!(
        recovered.call("read_commits", &json!({}), 3., &mut |_| Ok(()))?["commits"],
        json!([first])
    );
    Ok(())
}
#[test]
fn interrupted_append_tail_does_not_corrupt_prior_extent() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = config(dir.path(), 1024 * 1024);
    let mut s = Store::new(cfg.clone());
    let first = s.write(&[record("a", b"durable", json!({}))], "a", &mut |_| Ok(()))?[0].clone();
    let failed = s.write(&[record("b", b"torn", json!({}))], "b", &mut |phase| {
        if phase == "after_record" {
            Err(straw::Error::new("Hook", "crash"))
        } else {
            Ok(())
        }
    });
    assert!(failed.is_err());
    let mut new = Store::new(cfg);
    let later = new.write(&[record("c", b"later", json!({}))], "c", &mut |_| Ok(()))?[0].clone();
    assert_ne!(first["segment"]["path"], later["segment"]["path"]);
    assert_eq!(new.config.read(&first)?.payload.as_ref(), b"durable");
    Ok(())
}

/// A caller-provided stream can disagree with its length or swallow callback
/// errors. Publication must validate the stream before acknowledging it.
struct ScriptedPayload {
    declared: usize,
    chunks: Vec<Vec<u8>>,
    swallow_errors: bool,
    length_calls: std::sync::atomic::AtomicUsize,
    change_length: bool,
    later_chunks: Option<Vec<Vec<u8>>>,
    traversals: std::sync::atomic::AtomicUsize,
}
impl straw::payload::Payload for ScriptedPayload {
    fn len(&self) -> usize {
        let call = self
            .length_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.change_length && call > 0 {
            self.declared + 17
        } else {
            self.declared
        }
    }
    fn chunks(&self, _: usize, visit: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> {
        let traversal = self
            .traversals
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let chunks = if traversal > 0 {
            self.later_chunks.as_ref().unwrap_or(&self.chunks)
        } else {
            &self.chunks
        };
        for chunk in chunks {
            let result = visit(chunk);
            if !self.swallow_errors {
                result?;
            }
        }
        Ok(())
    }
}
fn scripted_payload(
    declared: usize,
    chunks: Vec<Vec<u8>>,
    swallow_errors: bool,
    change_length: bool,
) -> straw::payload::InputRecord {
    straw::payload::InputRecord {
        metadata: json!({"record_id":"stream","codec":"bytes.v1","metadata":{},"tokens":0}),
        payload: std::sync::Arc::new(ScriptedPayload {
            declared,
            chunks,
            swallow_errors,
            change_length,
            length_calls: std::sync::atomic::AtomicUsize::new(0),
            later_chunks: None,
            traversals: std::sync::atomic::AtomicUsize::new(0),
        }),
    }
}
#[test]
fn streamed_length_mismatch_is_rejected_before_any_write() -> Result<()> {
    for target in [0, 1024 * 1024] {
        for (declared, actual) in [(5, 3), (2, 3), (0, 3), (3, 0)] {
            let dir = tempfile::tempdir()?;
            let mut store = Store::new(config(dir.path(), target));
            let input = scripted_payload(declared, vec![vec![7; actual]], false, false);
            let mut writes = 0;
            let result = store.write_inputs(&[input], "stream", &mut |phase| {
                if phase == "after_segment_header" {
                    writes += 1;
                }
                Ok(())
            });
            assert!(
                matches!(result, Err(ref e) if e.kind == "ValueError"),
                "declared={declared}, actual={actual}: {result:?}"
            );
            assert_eq!(writes, 0);
        }
    }
    Ok(())
}
#[test]
fn streamed_callback_failure_cannot_be_swallowed_by_provider() -> Result<()> {
    for target in [0, 1024 * 1024] {
        let dir = tempfile::tempdir()?;
        let mut store = Store::new(config(dir.path(), target));
        let previous = store.write(
            &[record("previous", b"durable", json!({}))],
            "previous",
            &mut |_| Ok(()),
        )?;
        let input = scripted_payload(3, vec![vec![1], vec![2], vec![3]], true, false);
        let mut calls = 0;
        let result = store.write_inputs(&[input], "stream", &mut |phase| {
            if phase == "after_payload_chunk" {
                calls += 1;
                return straw::fail("StorageUnavailable", "injected stream callback failure");
            }
            Ok(())
        });
        assert!(
            matches!(result, Err(ref e) if e.kind == "StorageUnavailable"),
            "{result:?}"
        );
        assert_eq!(
            calls, 1,
            "No later callback can run after the first failure"
        );
        let refs = store.write(&[record("next", b"valid", json!({}))], "next", &mut |_| {
            Ok(())
        })?;
        assert_eq!(store.config.read(&refs[0])?.payload.as_ref(), b"valid");
        assert_eq!(
            store.config.read(&previous[0])?.payload.as_ref(),
            b"durable"
        );
    }
    Ok(())
}
#[test]
fn streamed_length_snapshot_is_used_for_the_frame_and_accounting() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut store = Store::new(config(dir.path(), 1024 * 1024));
    let input = scripted_payload(3, vec![b"abc".to_vec()], false, true);
    let refs = store.write_inputs(&[input], "stream", &mut |_| Ok(()))?;
    assert_eq!(store.config.read(&refs[0])?.payload.as_ref(), b"abc");
    assert_eq!(store.metrics["payload_bytes"], 3);
    Ok(())
}

#[test]
fn streamed_second_traversal_is_checked_before_footer_and_oversize_callback() -> Result<()> {
    for chunks in [vec![b"ab".to_vec()], vec![b"ab".to_vec(), b"cd".to_vec()]] {
        let dir = tempfile::tempdir()?;
        let mut store = Store::new(config(dir.path(), 1024 * 1024));
        let input = straw::payload::InputRecord {
            metadata: json!({"record_id":"changing","codec":"bytes.v1","metadata":{},"tokens":0}),
            payload: std::sync::Arc::new(ScriptedPayload {
                declared: 3,
                chunks: vec![b"abc".to_vec()],
                swallow_errors: true,
                change_length: false,
                length_calls: std::sync::atomic::AtomicUsize::new(0),
                traversals: std::sync::atomic::AtomicUsize::new(0),
                later_chunks: Some(chunks),
            }),
        };
        let mut callbacks = 0;
        let mut footers = 0;
        let result = store.write_inputs(&[input], "changing", &mut |phase| {
            if phase == "after_payload_chunk" {
                callbacks += 1;
            }
            if phase == "before_footer" {
                footers += 1;
            }
            Ok(())
        });
        assert!(matches!(result, Err(ref e) if e.kind == "ValueError"));
        assert_eq!(callbacks, 1);
        assert_eq!(footers, 0);
    }
    Ok(())
}

#[test]
fn streamed_empty_and_uneven_chunks_preserve_exact_bytes() -> Result<()> {
    for chunks in [vec![], vec![vec![], b"a".to_vec(), b"bc".to_vec(), vec![]]] {
        let dir = tempfile::tempdir()?;
        let mut store = Store::new(config(dir.path(), 1024 * 1024));
        let expected: Vec<_> = chunks.iter().flatten().copied().collect();
        let input = scripted_payload(expected.len(), chunks, false, false);
        let refs = store.write_inputs(&[input], "valid", &mut |_| Ok(()))?;
        assert_eq!(store.config.read(&refs[0])?.payload.as_ref(), expected);
    }
    Ok(())
}
