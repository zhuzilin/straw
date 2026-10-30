use super::*;

fn fixture() -> Result<(tempfile::TempDir, Store, Value, Value)> {
    let dir = tempfile::tempdir()?;
    let mut config = Config::new(
        dir.path().to_str().unwrap(),
        "proof",
        vec!["bytes.v1".into()],
        1 << 20,
        4 << 20,
        0,
    )?;
    config.online_gc = true;
    let mut store = Store::new(config);
    let record = |name| Record {
        metadata: json!({"record_id":name,"codec":"bytes.v1","metadata":{"task_id":"task","attempt_id":"attempt"},"tokens":0}),
        payload: b"payload".to_vec().into(),
    };
    let leaf = store.publish(
        &json!([{"records":[0],"dependencies":[]}]),
        vec![record("leaf")],
        "leaf",
        &mut |_| Ok(()),
    )?[0]
        .clone();
    let root = store.publish(
        &json!([{"records":[0],"dependencies":[leaf]}]),
        vec![record("root")],
        "root",
        &mut |_| Ok(()),
    )?[0]
        .clone();
    Ok((dir, store, leaf, root))
}
fn proof(config: &Config, root: &Value) -> Result<GraphProof> {
    let mut graph = GraphProof::new(config);
    Reader::new(config.clone()).validate_with_graph(
        root,
        Some("task"),
        Some("attempt"),
        &mut HashSet::new(),
        &mut graph,
    )?;
    Ok(graph)
}
fn corrupt(config: &Config, reference: &Value) -> Result<()> {
    let segment = &reference["manifest"]["segment"];
    let mut file = OpenOptions::new()
        .write(true)
        .open(config.root.join(field(segment, "path")?))?;
    file.seek(SeekFrom::Start(number(segment, "offset")))?;
    file.write_all(&[0; 12])?;
    Ok(())
}

#[test]
fn graph_proof_matches_exact_storage_and_complete_reference_identity() -> Result<()> {
    let (_dir, store, _leaf, root) = fixture()?;
    let graph = proof(&store.config, &root)?;
    assert!(graph.project(&store.config, &root, true)?.is_some());
    for field in ["records", "payload_bytes", "tokens"] {
        let mut other = root.clone();
        other[field] = json!(number(&other, field) + 1);
        assert!(graph.project(&store.config, &other, true)?.is_none());
    }
    for field in ["checksum", "run_id", "path"] {
        let mut other = root.clone();
        other["manifest"]["segment"][field] = json!("other");
        assert!(graph.project(&store.config, &other, true)?.is_none());
    }
    let mut config = store.config.clone();
    config.run_id = "other".into();
    assert!(graph.project(&config, &root, true)?.is_none());
    config = store.config.clone();
    config.root = config.root.join("other");
    assert!(graph.project(&config, &root, true)?.is_none());
    Ok(())
}

#[test]
fn graph_proof_projection_matches_original_graph_walk() -> Result<()> {
    let (_dir, store, _leaf, root) = fixture()?;
    let graph = proof(&store.config, &root)?;
    let (paths, actual) = graph.project(&store.config, &root, true)?.unwrap();
    let original_paths = store
        .config
        .catalog(|c| c.closure(&store.config, &root))?
        .unwrap();
    assert_eq!(paths, original_paths);
    let mut expected = HashSet::new();
    let mut pending = vec![root];
    let mut reader = Reader::new(store.config.clone());
    while let Some(reference) = pending.pop() {
        if !expected.insert(crate::gc::Catalog::staged(&reference)?) {
            continue;
        }
        if reference.get("manifest").is_some() {
            let manifest = reader.manifest(&reference)?;
            pending.extend(array(&manifest, "records")?.iter().cloned());
            pending.extend(array(&manifest, "dependencies")?.iter().cloned());
        }
    }
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn graph_proof_parallel_overlap_contains_every_successful_node() -> Result<()> {
    let (_dir, mut store, leaf, earlier) = fixture()?;
    let record = |name| Record {
        metadata: json!({"record_id":name,"codec":"bytes.v1","metadata":{"task_id":"task","attempt_id":"attempt"},"tokens":0}),
        payload: b"payload".to_vec().into(),
    };
    let groups = (0..64)
        .map(|i| json!({"records":[i],"dependencies":[leaf]}))
        .collect::<Vec<_>>();
    let mut roots = store.publish(
        &json!(groups),
        (0..64).map(|i| record(format!("branch-{i}"))).collect(),
        "branches",
        &mut |_| Ok(()),
    )?;
    roots.push(earlier);
    let root = store.publish(
        &json!([{"records":[0],"dependencies":roots}]),
        vec![record("wide".to_string())],
        "wide",
        &mut |_| Ok(()),
    )?[0]
        .clone();
    let graph = proof(&store.config, &root)?;
    assert_eq!(graph.nodes.len(), 67);
    let (paths, owners) = graph.project(&store.config, &root, true)?.unwrap();
    assert_eq!(owners.len(), 134); // every set and its one member
    assert_eq!(
        paths,
        store
            .config
            .catalog(|c| c.closure(&store.config, &root))?
            .unwrap()
    );
    Ok(())
}

#[test]
fn graph_proof_missing_child_strictly_falls_back_and_detects_corruption() -> Result<()> {
    let (_dir, store, leaf, root) = fixture()?;
    let mut graph = proof(&store.config, &root)?;
    graph.nodes.remove(&digest(&leaf)?);
    assert!(graph.project(&store.config, &root, true)?.is_none());
    corrupt(&store.config, &leaf)?;
    assert!(
        store
            .config
            .catalog(|c| c.consume_with_graph(
                &store.config,
                &[root],
                Some(&graph),
                &mut |_| Ok(())
            ))
            .is_err()
    );
    Ok(())
}

#[test]
fn graph_proof_failure_clears_scope_and_root_authorization_is_rechecked() -> Result<()> {
    let (_dir, store, _leaf, root) = fixture()?;
    let mut graph = proof(&store.config, &root)?;
    let mut reader = Reader::new(store.config.clone());
    let mut seen = graph.nodes.keys().cloned().collect();
    assert!(
        reader
            .validate_with_graph(&root, Some("wrong"), Some("attempt"), &mut seen, &mut graph)
            .is_err()
    );
    assert!(seen.is_empty() && graph.nodes.is_empty());
    reader.validate_with_graph(&root, Some("task"), Some("attempt"), &mut seen, &mut graph)?;
    let mut other = store.config.clone();
    other.root = other.root.join("other");
    assert!(
        Reader::new(other)
            .validate_with_graph(&root, None, None, &mut seen, &mut graph)
            .is_err()
    );
    assert!(seen.is_empty() && graph.nodes.is_empty());
    Ok(())
}

#[test]
fn graph_proof_cannot_retain_a_now_tombstoned_pack() -> Result<()> {
    let (_dir, mut store, _leaf, root) = fixture()?;
    let graph = proof(&store.config, &root)?;
    store.close()?;
    store.config.catalog(|c| {
        c.prune(&store.config, "staged:", &HashSet::new(), &mut |_| Ok(()))?;
        c.collect(&store.config, &mut |_| Ok(()))?;
        let error = c
            .retain_many_with_graph(
                &store.config,
                &[("late".into(), vec![root])],
                Some(&graph),
                &mut |_| Ok(()),
            )
            .unwrap_err();
        assert_eq!(error.kind, "InvalidReference");
        Ok(())
    })?;
    Ok(())
}
