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
    let result = q.store.publish(
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
