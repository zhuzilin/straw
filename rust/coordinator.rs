//! Single-owner durable scheduling and accepted-result state machine.
use crate::journal::Journal;
use crate::queue_state;
use crate::store::{Config, GraphProof, Reader, Record, Store, mkdir, sync, sync_dir};
use crate::{Error, Fault, Result, array, bytes, checked_sum, digest, fail, field, number, uid};
use serde_json::{Value, json};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap, HashSet},
    fs::OpenOptions,
    io::Write,
    time::Instant,
};

/// A coordinator owns its journal and business state. Inspect them through
/// immutable views and perform queue transitions through [`Self::call`].
/// Use an independent [`Store`] for producer publication.
///
/// ```no_run
/// # use straw::coordinator::Coordinator;
/// fn inspect(queue: &Coordinator) {
///     let _: &serde_json::Value = queue.state();
///     let _: &serde_json::Value = queue.limits();
///     let _: &str = queue.epoch();
///     let _sequence = queue.journal().sequence;
///     let _config = queue.store().config.clone();
/// }
/// ```
///
/// Direct replacement cannot bypass replay or its derived indexes:
/// ```compile_fail
/// # use straw::coordinator::Coordinator;
/// fn replace_state(queue: &mut Coordinator) {
///     queue.state = serde_json::json!({});
/// }
/// ```
/// ```compile_fail
/// # use straw::coordinator::Coordinator;
/// fn mutate_view(queue: &mut Coordinator) {
///     queue.state()["tasks"] = serde_json::json!({});
/// }
/// ```
///
/// A journal view does not expose append or replacement:
/// ```compile_fail
/// # use straw::coordinator::Coordinator;
/// fn bypass_commit(queue: &mut Coordinator) {
///     queue.journal().append(&serde_json::json!([{"type":"Sealed"}]), &mut |_| Ok(())).unwrap();
/// }
/// ```
/// ```compile_fail
/// # use straw::{coordinator::Coordinator, journal::Journal};
/// fn replace_journal(queue: &mut Coordinator, replacement: Journal) {
///     let _ = std::mem::replace(&mut queue.journal, replacement);
/// }
/// ```
///
/// The owned storage identity cannot be changed through its view:
/// ```compile_fail
/// # use straw::coordinator::Coordinator;
/// fn replace_storage_root(queue: &mut Coordinator) {
///     queue.store().config.root = std::path::PathBuf::new();
/// }
/// ```
pub struct Coordinator {
    pub(crate) store: Store,
    pub(crate) journal: Journal,
    state: Value,
    limits: Value,
    epoch: String,
    pub recovery_seconds: f64,
    pending: BTreeMap<(Reverse<i64>, i64, usize), String>,
    pending_keys: HashMap<String, (Reverse<i64>, i64, usize)>,
    next_pending_order: usize,
    leased: HashSet<String>,
    usage_cache: Option<Value>,
    deadlines: HashMap<String, f64>,
    owners: HashMap<String, String>,
    queue_id: String,
    storage_prefix: String,
    lease_seconds: f64,
    sealed: bool,
    draining: bool,
    storage_failed: bool,
    counters: Value,
    complete_seconds: Vec<f64>,
    progress_cache: crate::queue_progress::Cache,
    pending_consumption: Vec<(String, Vec<Value>)>,
}
// Storage retention cannot borrow queue business state or its journal.
// The typed catalog-retention operation supplies the retained-root evidence.
fn retain_commit(
    config: &Config,
    storage_prefix: &str,
    reactivated: &[(String, Value)],
    roots: &[(String, Vec<Value>)],
    storage_failed: &mut bool,
    fault: &mut Fault<'_>,
    graph: Option<&GraphProof>,
) -> Result<()> {
    let retained = (|| {
        let verified = config.verify_queue_reactivation(storage_prefix, reactivated, fault)?;
        config.catalog(|catalog| {
            catalog.retain_queue_verified(config, storage_prefix, &verified, fault)?;
            catalog.retain_queue_many_with_graph(config, storage_prefix, roots, graph, fault)
        })
    })();
    if retained
        .as_ref()
        .is_err_and(|e| e.kind == "StorageUnavailable")
    {
        *storage_failed = true;
    }
    retained.map(|_| ())
}
fn commit_error(error: crate::queue_commit::Error<Error>, events: &Value) -> Error {
    match error {
        crate::queue_commit::Error::Append(error) => crate::journal::owner_error(error),
        crate::queue_commit::Error::Apply(error) => match error {
            crate::queue_application::Error::Before(error)
            | crate::queue_application::Error::After(error) => error,
            crate::queue_application::Error::Body(failure) => {
                let event = events
                    .as_array()
                    .and_then(|events| events.get(failure.index))
                    .unwrap_or(&Value::Null);
                history_error(failure.error, event)
            }
        },
    }
}

fn terminal(v: &Value) -> bool {
    matches!(v.as_str(), Some("completed" | "failed" | "cancelled"))
}
fn boolean(v: &Value, k: &str) -> bool {
    v[k].as_bool().unwrap_or(false)
}
fn vals(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_object().into_iter().flat_map(|o| o.values())
}
fn task_error(error: crate::queue_task::Error) -> Error {
    Error::new(
        "UnsafeRecovery",
        match error {
            crate::queue_task::Error::Task => "Journal task target is missing or invalid",
            crate::queue_task::Error::Counter => "Journal task failure counter is exhausted",
        },
    )
}
fn validate_scheduling(value: &Value) -> Result<()> {
    for key in ["priority", "scheduling_key"] {
        if value.get(key).is_some_and(|v| v.as_i64().is_none()) {
            return fail(
                "ValueError",
                format!("{key} must be a signed 64-bit integer"),
            );
        }
    }
    Ok(())
}
fn reader_error(error: crate::queue_reader::Error) -> Error {
    match error {
        crate::queue_reader::Error::Identity => {
            Error::new("InvalidReference", "Missing string attempt_id")
        }
        crate::queue_reader::Error::State => Error::new(
            "UnsafeRecovery",
            "Reader protection record is missing or invalid",
        ),
    }
}
fn receipt_error(error: crate::queue_receipt::Error) -> Error {
    match error {
        crate::queue_receipt::Error::Identity => {
            Error::new("InvalidReference", "Missing string submission_id")
        }
        crate::queue_receipt::Error::State => Error::new(
            "UnsafeRecovery",
            "Receipt or producer state is missing or invalid",
        ),
    }
}
fn state_error(error: queue_state::Error) -> Error {
    match error {
        queue_state::Error::Identity("task ID") => {
            Error::new("CorruptData", "Expected identity string")
        }
        queue_state::Error::Identity(name) => {
            Error::new("InvalidReference", format!("Missing string {name}"))
        }
        queue_state::Error::Target => task_error(crate::queue_task::Error::Task),
        queue_state::Error::Task(error) => task_error(error),
    }
}
fn transition_error(error: crate::queue_transition::Error) -> Error {
    match error {
        crate::queue_transition::Error::Task(error) => state_error(error),
        crate::queue_transition::Error::Reader(error) => reader_error(error),
        crate::queue_transition::Error::Receipt(error) => receipt_error(error),
    }
}
fn admin_error(error: crate::queue_admin::Error) -> Error {
    match error {
        crate::queue_admin::Error::Task(error) => state_error(error),
        crate::queue_admin::Error::Transition(error) => transition_error(error),
        crate::queue_admin::Error::Receipt(error) => receipt_error(error),
        crate::queue_admin::Error::Reader(error) => reader_error(error),
        crate::queue_admin::Error::Array(name) => {
            Error::new("InvalidReference", format!("Missing array {name}"))
        }
        crate::queue_admin::Error::State => {
            Error::new("UnsafeRecovery", "Queue replay table is missing or invalid")
        }
        crate::queue_admin::Error::Order => {
            Error::new("UnsafeRecovery", "Queue scheduling order is exhausted")
        }
    }
}
struct ReplayCaches<'a> {
    pending: &'a mut BTreeMap<(Reverse<i64>, i64, usize), String>,
    pending_keys: &'a mut HashMap<String, (Reverse<i64>, i64, usize)>,
    leased: &'a mut HashSet<String>,
    usage: &'a mut Option<Value>,
    deadlines: &'a mut HashMap<String, f64>,
}
impl crate::queue_replay::Deadlines for ReplayCaches<'_> {
    fn remove(&mut self, id: &String) {
        self.deadlines.remove(id);
    }
}
impl crate::queue_history::Caches for ReplayCaches<'_> {
    fn before(&mut self, id: &String, task: &Value) {
        if let Some(key) = self.pending_keys.remove(id) {
            self.pending.remove(&key);
        }
        self.leased.remove(id);
        if self
            .usage
            .as_mut()
            .is_some_and(|usage| Coordinator::adjust_active(usage, task, false).is_none())
        {
            *self.usage = None;
        }
    }
    fn after(&mut self, id: &String, task: &Value) {
        if task["state"] == "pending" {
            let key = (
                Reverse(task["spec"]["priority"].as_i64().unwrap_or(0)),
                task["spec"]["scheduling_key"].as_i64().unwrap_or(0),
                number(task, "pending_order") as usize,
            );
            self.pending.insert(key, id.clone());
            self.pending_keys.insert(id.clone(), key);
        }
        if task["state"] == "leased" {
            self.leased.insert(id.clone());
        }
        if self
            .usage
            .as_mut()
            .is_some_and(|usage| Coordinator::adjust_active(usage, task, true).is_none())
        {
            *self.usage = None;
        }
    }
    fn event(&mut self, kind: &str, event: &Value) {
        if kind == "Completed" {
            *self.usage = self.usage.take().and_then(|mut usage| {
                for (key, field) in [
                    ("records", "records"),
                    ("bytes", "payload_bytes"),
                    ("tokens", "tokens"),
                ] {
                    let n = number(&event["receipt"]["result_ref"], field);
                    usage["accepted_unprocessed"][key] = json!(crate::bounds::add(
                        number(&usage["accepted_unprocessed"], key),
                        n
                    )?);
                    usage[key] = json!(crate::bounds::add(number(&usage, key), n)?);
                }
                Some(usage)
            });
        } else if matches!(kind, "ConsumerState" | "BatchReady" | "Checkpoint") {
            *self.usage = None;
        }
    }
}

struct RecoveryBookkeeping<'a> {
    prefix: &'a str,
    pending: &'a mut Vec<(String, Vec<Value>)>,
}

struct ApplyHooks<'a, 'f>(&'a mut Fault<'f>);
impl crate::wal_sink::Hooks<crate::wal_file::NativeFile<'_>> for ApplyHooks<'_, '_> {
    fn point(&mut self, _file: &mut crate::wal_file::NativeFile<'_>, point: &str) -> Result<()> {
        (self.0)(point)
    }
}
impl crate::queue_application::Hooks for ApplyHooks<'_, '_> {
    type Error = Error;
    fn before(&mut self) -> Result<()> {
        (self.0)("before_commit_apply")
    }
    fn after(&mut self) -> Result<()> {
        (self.0)("after_commit_apply")
    }
}
impl crate::queue_history::Recovery for RecoveryBookkeeping<'_> {
    type Error = Error;
    fn before(&mut self, index: usize, transaction: &Value) -> Result<()> {
        crate::queue_handoff::recover_into(
            &mut RootDigest,
            self.prefix,
            index as u64,
            transaction,
            self.pending,
        )
        .map_err(handoff_error)
    }
}

fn history_error(error: crate::queue_history::Error, event: &Value) -> Error {
    match error {
        crate::queue_history::Error::Prepare(error) => match error {
            crate::queue_event::Error::StringField(name) => {
                Error::new("InvalidReference", format!("Missing string {name}"))
            }
            crate::queue_event::Error::ArrayField(name) => {
                Error::new("InvalidReference", format!("Missing array {name}"))
            }
            crate::queue_event::Error::Identity => {
                Error::new("CorruptData", "Expected identity string")
            }
        },
        crate::queue_history::Error::Target => task_error(crate::queue_task::Error::Task),
        crate::queue_history::Error::Receipt(error) => admin_error(error),
        crate::queue_history::Error::Transaction => {
            Error::new("CorruptData", "Invalid transaction")
        }
        crate::queue_history::Error::Body(error) => match error {
            crate::queue_replay::Error::Admin(error) => admin_error(error),
            crate::queue_replay::Error::Transition(error) => transition_error(error),
            crate::queue_replay::Error::Metadata(error) => match error {
                crate::queue_metadata::Error::Event => {
                    Error::new("InvalidReference", "Invalid queue metadata event")
                }
                crate::queue_metadata::Error::State => Error::new(
                    "UnsafeRecovery",
                    "Queue metadata path is missing or invalid",
                ),
            },
            crate::queue_replay::Error::Unknown => Error::new(
                "UnsafeRecovery",
                format!(
                    "Unknown journal event type: {}",
                    event["type"].as_str().unwrap_or("")
                ),
            ),
        },
    }
}

impl Coordinator {
    pub fn store(&self) -> &Store {
        &self.store
    }
    pub fn journal(&self) -> &Journal {
        &self.journal
    }
    pub fn state(&self) -> &Value {
        &self.state
    }
    pub fn limits(&self) -> &Value {
        &self.limits
    }
    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    pub fn open(config: Config, options: &Value, fault: &mut Fault<'_>) -> Result<Self> {
        let started = Instant::now();
        if options["exclusive_owner"]
            .as_str()
            .is_none_or(|s| s.is_empty())
        {
            return fail("UnsafeRecovery", "External single-owner guarantee required");
        }
        let queue_id = options["queue_id"]
            .as_str()
            .unwrap_or("rollout")
            .to_string();
        let lease_seconds = options["lease_seconds"].as_f64().unwrap_or(300.);
        if lease_seconds <= 0. || queue_id.is_empty() {
            return fail("ValueError", "Invalid lease duration/queue ID");
        }
        let defaults = json!({"pending_tasks":10000,"inflight_tasks":256,"control_tasks":1,"accepted_records":100000,"accepted_bytes":68719476736u64,"accepted_tokens":100000000,"ready_bytes":68719476736u64,"max_result_bytes":4294967296u64,"max_result_tokens":100000000,"metadata_bytes":262144});
        let limits = options
            .get("limits")
            .filter(|v| !v.is_null())
            .unwrap_or(&defaults)
            .clone();
        if vals(&limits).any(|v| v.as_u64().is_none_or(|n| n == 0)) {
            return fail("ValueError", "Queue limits must be positive");
        }
        let mut codecs = config.codecs.iter().cloned().collect::<Vec<_>>();
        codecs.sort();
        let identity = json!({"run_id":config.run_id,"queue_id":queue_id,"version":1,"limits":limits,"lease_seconds":lease_seconds,"profile":options["profile"].as_str().unwrap_or("local"),"codecs":codecs});
        let control_root = if boolean(options, "namespace") {
            config.root.join("queues").join(digest(&json!(queue_id))?)
        } else {
            config.root.clone()
        };
        let storage_prefix = format!("queue:{}:", digest(&json!(control_root))?);
        let run = control_root.join("run.json");
        mkdir(&control_root, fault)?;
        let recover = boolean(options, "recover");
        if recover {
            let prior: Value = serde_json::from_reader(std::fs::File::open(&run)?)?;
            if prior != identity {
                return fail("UnsafeRecovery", "Run identity/configuration mismatch");
            }
        } else {
            let mut f = OpenOptions::new().write(true).create_new(true).open(&run)?;
            f.write_all(&bytes(&identity)?)?;
            sync(&f, fault)?;
            sync_dir(&control_root, fault)?;
        }
        let journal = Journal::open(&control_root, &identity, recover, false, fault)?;
        let mut result = Self {
            store: Store::new(config),
            journal,
            state: Value::Null,
            limits,
            epoch: String::new(),
            recovery_seconds: 0.,
            pending: BTreeMap::new(),
            pending_keys: HashMap::new(),
            next_pending_order: 0,
            leased: HashSet::new(),
            usage_cache: None,
            deadlines: HashMap::new(),
            owners: HashMap::new(),
            queue_id,
            storage_prefix,
            lease_seconds,
            sealed: false,
            draining: false,
            storage_failed: false,
            counters: json!({}),
            complete_seconds: vec![],
            progress_cache: crate::queue_progress::Cache::new(),
            pending_consumption: Vec::new(),
        };
        let restored = crate::queue_application::restore(
            result.journal.state_mut(),
            &mut ReplayCaches {
                pending: &mut result.pending,
                pending_keys: &mut result.pending_keys,
                leased: &mut result.leased,
                usage: &mut result.usage_cache,
                deadlines: &mut result.deadlines,
            },
            &mut RecoveryBookkeeping {
                prefix: &result.storage_prefix,
                pending: &mut result.pending_consumption,
            },
        )
        .map_err(|failure| match failure {
            crate::queue_application::RestoreError::Unavailable => Error::new(
                "UnsafeRecovery",
                "Journal unavailable; stop owner and recover",
            ),
            crate::queue_application::RestoreError::History(failure) => match failure.cause {
                crate::queue_history::ReplayCause::Bookkeeping(error) => error,
                crate::queue_history::ReplayCause::Application(application) => {
                    let event = result
                        .journal
                        .transactions
                        .get(failure.index)
                        .and_then(Value::as_array)
                        .and_then(|events| events.get(application.index))
                        .unwrap_or(&Value::Null);
                    history_error(application.error, event)
                }
            },
        })?;
        result.state = restored.state;
        result.epoch = restored.epoch;
        result.next_pending_order = restored.order;
        result.sealed = restored.sealed;
        let revoked = vals(&result.state["tasks"])
            .filter(|t| t["state"] == "leased")
            .map(|t| t["spec"]["task_id"].clone())
            .collect::<Vec<_>>();
        result.commit(
            json!([{"type":"Epoch","epoch":uid(),"revoked":revoked}]),
            fault,
        )?;
        result.recovery_seconds = started.elapsed().as_secs_f64();
        Ok(result)
    }
    fn commit(&mut self, events: Value, fault: &mut Fault<'_>) -> Result<()> {
        self.commit_with_graph(events, fault, None)
    }
    fn commit_with_graph(
        &mut self,
        mut events: Value,
        fault: &mut Fault<'_>,
        graph: Option<&GraphProof>,
    ) -> Result<()> {
        // No newer commit/GC may abandon the owner that protects an unfinished
        // staging handoff. Recovery also reaches this barrier before its epoch.
        self.finish_consumption(fault)?;
        let mut refs = HashMap::new();
        let mut reactivated = HashMap::new();
        if self.store.config.gc_enabled()? {
            references(&events, &mut refs)?;
            // Consumer restore can make retired receipts/batches live again.
            // Their roots are implicit in the cursor, not in the event refs.
            let training = crate::queue_commit::restore_event(events.as_array().unwrap());
            if let Some(index) = training {
                let state = events[index]["state"].clone();
                let progress = self.progress(&state["progress_ref"])?;
                self.training_roots(&state, &progress, &mut reactivated)?;
                let prefix = number(&state, "processed_cursor");
                let positions = array(&progress, "processed_positions")?;
                if crate::queue_commit::retention_needed(
                    number(&self.state, "retired_prefix"),
                    array(&self.state, "retired_positions")?,
                    prefix,
                    positions,
                ) {
                    events.as_array_mut().unwrap().push(json!({
                        "type":"RetentionAdvanced", "prefix":prefix,
                        "positions":positions
                    }));
                }
            }
        }
        let requests = crate::queue_commit::owner_requests(&self.storage_prefix, refs);
        let roots = requests.roots;
        let references = requests.references;
        let reactivated =
            crate::queue_commit::reactivation_requests(&self.storage_prefix, reactivated);
        let handoff = crate::queue_handoff::attach(
            &mut RootDigest,
            &self.storage_prefix,
            self.journal.sequence,
            events.as_array_mut().unwrap(),
            references,
        )?;
        retain_commit(
            &self.store.config,
            &self.storage_prefix,
            &reactivated,
            &roots,
            &mut self.storage_failed,
            fault,
            graph,
        )?;
        crate::queue_commit::persist_and_apply::<_, crate::wal_file::NativeLibrary, _, _>(
            &mut self.journal,
            &mut self.state,
            &mut self.epoch,
            &mut self.next_pending_order,
            &mut self.sealed,
            &mut self.pending_consumption,
            handoff,
            &events,
            &mut ReplayCaches {
                pending: &mut self.pending,
                pending_keys: &mut self.pending_keys,
                leased: &mut self.leased,
                usage: &mut self.usage_cache,
                deadlines: &mut self.deadlines,
            },
            &mut ApplyHooks(fault),
        )
        .map_err(|error| commit_error(error, &events))?;
        fault("before_staging_consume")?;
        finish_consumption_with_graph(
            &self.store.config,
            &self.storage_prefix,
            &mut self.pending_consumption,
            fault,
            graph,
        )
    }
    fn finish_consumption(&mut self, fault: &mut Fault<'_>) -> Result<()> {
        finish_consumption(
            &self.store.config,
            &self.storage_prefix,
            &mut self.pending_consumption,
            fault,
        )
    }
    fn adjust_active(usage: &mut Value, task: &Value, add: bool) -> Option<()> {
        if task.is_null() || task["state"] != "leased" {
            return Some(());
        }
        let control = boolean(&task["spec"], "control");
        let key = if control {
            "control_inflight"
        } else {
            "inflight"
        };
        let change = |n: u64, amount: u64| crate::bounds::adjust(n, amount, add);
        usage[key] = json!(change(number(usage, key), 1)?);
        if !control {
            for key in ["records", "bytes", "tokens"] {
                usage[key] = json!(change(
                    number(usage, key),
                    number(&task["spec"], &format!("estimated_{key}"))
                )?);
            }
        }
        Some(())
    }
    fn request(
        &self,
        operation: &str,
        id: &str,
        content: &Value,
    ) -> Result<(String, String, Option<Value>)> {
        let key = Self::request_key(operation, id)?;
        let hash = digest(content)?;
        let prior =
            crate::request::lookup(self.state["requests"].as_object().unwrap(), &key, &hash)
                .map_err(|_| {
                    Error::new(
                        "IdempotencyConflict",
                        format!("{operation} request ID reused with different content"),
                    )
                })?;
        Ok((key, hash, prior))
    }
    fn request_key(operation: &str, id: &str) -> Result<String> {
        if id.is_empty() || id.len() > 1024 {
            return fail("ValueError", "A bounded stable request ID is required");
        }
        Ok(String::from_utf8(bytes(&json!([operation, id]))?).unwrap())
    }
    fn event(
        kind: &str,
        request: &(String, String, Option<Value>),
        response: Value,
        mut extra: Value,
    ) -> Value {
        extra["type"] = json!(kind);
        extra["request_key"] = json!(request.0);
        extra["request_digest"] = json!(request.1);
        extra["response"] = response;
        extra
    }
    fn count(&mut self, key: &str) {
        self.counters[key] = json!(number(&self.counters, key) + 1);
    }
    fn validate_graph(
        &mut self,
        reference: &Value,
        task: Option<&str>,
        attempt: Option<&str>,
        graph: &mut GraphProof,
    ) -> Result<Vec<Value>> {
        let mut reader = Reader::new(self.store.config.clone());
        let result =
            reader.validate_with_graph(reference, task, attempt, &mut graph.validated(), graph);
        if result
            .as_ref()
            .is_err_and(|error| error.kind == "StorageUnavailable")
        {
            self.storage_failed = true;
        }
        result
    }
    fn valid_lease(&mut self, lease: &Value, now: f64) -> Result<Value> {
        let id = field(lease, "task_id")?;
        let task = &self.state["tasks"][id];
        if task.is_null()
            || task["state"] != "leased"
            || task["lease"] != *lease
            || lease["coordinator_epoch"] != self.epoch
        {
            self.count("stale_attempts");
            return fail("StaleAttempt", "Lease is not current authorization");
        }
        if self.deadlines.get(id).copied().unwrap_or(0.) <= now {
            return fail("LeaseExpired", "Lease expired");
        };
        Ok(task.clone())
    }
    fn progress(&mut self, reference: &Value) -> Result<Value> {
        crate::queue_progress::load(
            &mut self.progress_cache,
            &mut ProgressReader {
                config: &self.store.config,
                validated_members: None,
                storage_failed: &mut self.storage_failed,
            },
            reference,
        )
        .map_err(progress_error)
    }
    fn progress_from_members(&mut self, reference: &Value, members: &[Value]) -> Result<Value> {
        crate::queue_progress::load(
            &mut self.progress_cache,
            &mut ProgressReader {
                config: &self.store.config,
                validated_members: Some(members),
                storage_failed: &mut self.storage_failed,
            },
            reference,
        )
        .map_err(progress_error)
    }
    fn covered(&self, batch: &Value) -> bool {
        vals(&self.state["checkpoints"]).any(|c| {
            c["batch_ids"]
                .as_array()
                .is_some_and(|a| a.contains(&batch["batch_id"]))
        })
    }
    pub fn usage(&mut self) -> Result<Value> {
        if !crate::queue_application::permitted(&self.journal, self.journal.closed(), "usage") {
            return fail(
                "UnsafeRecovery",
                "Coordinator unavailable; stop owner and recover",
            );
        }
        if let Some(usage) = &self.usage_cache {
            return Ok(usage.clone());
        }
        let state = self.state["consumers"]["training"].clone();
        let prefix = number(&state, "processed_cursor");
        let progress = self.progress(&state["progress_ref"])?;
        let processed: HashSet<u64> = array(&progress, "processed_positions")?
            .iter()
            .filter_map(|v| v.as_u64())
            .collect();
        let finished = array(&progress, "finished_batches")?;
        let accepted = array(&self.state, "commits")?
            .iter()
            .filter(|r| {
                number(r, "position") >= prefix && !processed.contains(&number(r, "position"))
            })
            .collect::<Vec<_>>();
        let active = vals(&self.state["tasks"])
            .filter(|t| t["state"] == "leased" && !boolean(&t["spec"], "control"))
            .collect::<Vec<_>>();
        let ready = vals(&self.state["batches"])
            .filter(|b| {
                boolean(b, "ready") && !finished.contains(&b["batch_id"]) && !self.covered(b)
            })
            .collect::<Vec<_>>();
        let mut usage = json!({"accepted_unprocessed":{},"train_ready":{},"inflight":active.len(),"control_inflight":vals(&self.state["tasks"]).filter(|t|t["state"]=="leased"&&boolean(&t["spec"],"control")).count()});
        for (key, field) in [
            ("records", "records"),
            ("bytes", "payload_bytes"),
            ("tokens", "tokens"),
        ] {
            let n = checked_sum(
                accepted.iter().map(|r| number(&r["result_ref"], field)),
                "ValueError",
            )?;
            let r = checked_sum(
                ready.iter().map(|b| number(&b["ready_ref"], field)),
                "ValueError",
            )?;
            usage["accepted_unprocessed"][key] = json!(n);
            usage["train_ready"][key] = json!(r);
            usage[key] = json!(checked_sum(
                std::iter::once(n).chain(
                    active
                        .iter()
                        .map(|t| number(&t["spec"], &format!("estimated_{key}")))
                ),
                "ValueError"
            )?);
        }
        usage["ready_bytes"] = usage["train_ready"]["bytes"].clone();
        self.usage_cache = Some(usage.clone());
        Ok(usage)
    }
    fn consumer(&self, id: &str, token: &str) -> Result<()> {
        if token.is_empty() || self.owners.get(id).is_none_or(|t| t != token) {
            return fail("StaleAttempt", "Consumer ownership is stale");
        }
        Ok(())
    }
    fn consumer_state(&self, args: &Value) -> Result<Value> {
        let fetched = number(args, "fetch_cursor");
        let processed = number(args, "processed_cursor");
        if processed > fetched || fetched > array(&self.state, "commits")?.len() as u64 {
            return fail("ValueError", "Consumer cursor order is invalid");
        }
        let mut state = json!({"version":1,"state_ref":args["state_ref"],"fetch_cursor":fetched,"processed_cursor":processed});
        if !args["progress_ref"].is_null() {
            state["progress_ref"] = args["progress_ref"].clone();
        }
        Ok(state)
    }
    fn validate_consumer_state(&mut self, state: &Value, graph: &mut GraphProof) -> Result<()> {
        self.validate_graph(&state["state_ref"], None, None, graph)?;
        if !state["progress_ref"].is_null() {
            let members = self.validate_graph(&state["progress_ref"], None, None, graph)?;
            let progress = self.progress_from_members(&state["progress_ref"], &members)?;
            let processed = number(state, "processed_cursor");
            let fetched = number(state, "fetch_cursor");
            if array(&progress, "processed_positions")?
                .iter()
                .any(|p| p.as_u64().is_none_or(|p| p < processed || p >= fetched))
                || array(&progress, "finished_batches")?
                    .iter()
                    .any(|b| !boolean(&self.state["batches"][b.as_str().unwrap_or("")], "ready"))
            {
                return fail(
                    "InvalidReference",
                    "Progress outside cursor range or unknown batch",
                );
            }
        }
        Ok(())
    }
    pub fn call(
        &mut self,
        method: &str,
        args: &Value,
        now: f64,
        fault: &mut Fault<'_>,
    ) -> Result<Value> {
        self.call_with_graph(method, args, now, fault, None)
    }
    pub(crate) fn call_with_graph(
        &mut self,
        method: &str,
        args: &Value,
        now: f64,
        fault: &mut Fault<'_>,
        graph: Option<&mut GraphProof>,
    ) -> Result<Value> {
        let mut local_graph = GraphProof::new(&self.store.config);
        let graph = graph.unwrap_or(&mut local_graph);
        graph.check_storage(&self.store.config)?;
        if !crate::queue_application::permitted(&self.journal, self.journal.closed(), method) {
            return fail(
                "UnsafeRecovery",
                "Coordinator unavailable; stop owner and recover",
            );
        }
        if method != "state" {
            self.finish_consumption(fault)?;
        }
        match method {
            "release_task_reads" => {
                for lease in array(args, "leases")? {
                    let prior = &self.state["readers"][field(lease, "attempt_id")?];
                    if !prior.is_null() && prior["lease"] != *lease {
                        return fail("StaleAttempt", "Reader lease identity mismatch");
                    }
                }
                self.commit_with_graph(
                    json!([{"type":"ReaderDone","leases":args["leases"]}]),
                    fault,
                    Some(graph),
                )?;
                Ok(Value::Null)
            }
            "collect_garbage" => self.collect_garbage(fault),
            "release_checkpoint" => {
                self.commit_with_graph(json!([{"type":"CheckpointReleased","checkpoint_id":field(args,"checkpoint_id")?}]), fault, Some(graph))?;
                Ok(Value::Null)
            }
            "submit_tasks" => {
                let tasks = array(args, "tasks")?;
                let producer = args["producer_id"].as_str();
                if producer.is_some() == args["producer_state"].is_null() {
                    return fail("ValueError", "Producer ID/state must be provided together");
                }
                let content = if producer.is_some() {
                    json!({"tasks":tasks,"producer_id":producer,"producer_state":args["producer_state"]})
                } else {
                    json!(tasks)
                };
                let request = self.request("submit", field(args, "request_id")?, &content)?;
                if let Some(response) = request.2 {
                    self.count("duplicate_submissions");
                    return Ok(response);
                }
                if self.sealed || self.draining {
                    return fail("ValueError", "Input is sealed or draining");
                }
                if self.storage_failed {
                    return fail(
                        "StorageUnavailable",
                        "Scheduling paused after storage failure",
                    );
                }
                if !crate::queue_admin::has_capacity(self.next_pending_order, tasks.len()) {
                    return fail("ValueError", "Queue scheduling order is exhausted");
                }
                let mut ids = vec![];
                let mut unique = HashSet::new();
                for t in tasks {
                    validate_scheduling(t)?;
                    let id = field(t, "task_id")?;
                    if !unique.insert(id) || !self.state["tasks"][id].is_null() {
                        return fail("IdempotencyConflict", "Task IDs must be new within queue");
                    }
                    if id.is_empty() || id.len() > 1024 || number(t, "max_attempts") == 0 {
                        return fail("ValueError", "Invalid task identity/retry limit");
                    }
                    for key in ["records", "bytes", "tokens"] {
                        let estimate = t[format!("estimated_{key}")].as_u64().ok_or_else(|| {
                            Error::new("ValueError", "Task estimate must be nonnegative")
                        })?;
                        if boolean(t, "control") && estimate != 0 {
                            return fail("ValueError", "Control tasks cannot reserve production");
                        };
                    }
                    if !t["input_ref"].is_null() {
                        self.validate_graph(&t["input_ref"], None, None, graph)?;
                    }
                    ids.push(json!(id));
                }
                self.commit_with_graph(json!([Self::event("Submitted",&request,json!(ids),json!({"tasks":tasks,"producer_id":producer,"producer_state":args["producer_state"]}))]),fault, Some(graph))?;
                Ok(json!(ids))
            }
            "acquire" => {
                let worker = field(args, "worker_id")?;
                let max = args["max_tasks"].as_u64().unwrap_or(1);
                if worker.is_empty() || worker.len() > 1024 || max == 0 {
                    return fail("ValueError", "Invalid worker identity/max tasks");
                }
                let expired = self
                    .deadlines
                    .iter()
                    .filter(|(_, d)| **d <= now)
                    .map(|(id, _)| json!(id))
                    .collect::<Vec<_>>();
                if !expired.is_empty() {
                    self.commit_with_graph(
                        json!([{"type":"Expired","task_ids":expired}]),
                        fault,
                        Some(graph),
                    )?;
                    self.counters["expired_leases"] =
                        json!(number(&self.counters, "expired_leases") + expired.len() as u64);
                }
                if self.draining {
                    return Ok(json!({"status":"draining","assignments":[]}));
                }
                if self.storage_failed {
                    return fail(
                        "StorageUnavailable",
                        "Scheduling paused after storage failure",
                    );
                }
                if self.sealed && self.pending.is_empty() && self.leased.is_empty() {
                    return Ok(json!({"status":"end_of_input","assignments":[]}));
                }
                let control = boolean(args, "control");
                let ids = args["task_ids"].as_array();
                let prefix = args["task_prefix"].as_str();
                let mut assignments = vec![];
                let candidates: Vec<&String> = if let Some(ids) = ids {
                    let mut selected = ids
                        .iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(|id| self.pending_keys.get(id))
                        .filter_map(|p| self.pending.get_key_value(p))
                        .collect::<Vec<_>>();
                    selected.sort_by_key(|(position, _)| *position);
                    // A filter is a set: repeated IDs must not generate two
                    // leases for one task or consume additional batch slots.
                    let keys = selected
                        .iter()
                        .map(|(p, _)| (p.0.0, p.1, p.2))
                        .collect::<Vec<_>>();
                    selected
                        .into_iter()
                        .zip(crate::selection::first_occurrences(&keys))
                        .filter(|(_, first)| *first)
                        .map(|((_, id), _)| id)
                        .collect()
                } else {
                    self.pending.values().collect()
                };
                for id in candidates {
                    if prefix.is_some_and(|p| !id.starts_with(p)) {
                        continue;
                    }
                    let task = &self.state["tasks"][id];
                    let spec = &task["spec"];
                    if ids.is_some_and(|v| !v.contains(&json!(id)))
                        || task["state"] != "pending"
                        || boolean(spec, "control") != control
                    {
                        continue;
                    }
                    let lease = json!({"run_id":self.store.config.run_id,"queue_id":self.queue_id,"task_id":id,"attempt_id":uid(),"generation":number(task,"generation")+1,"coordinator_epoch":self.epoch,"token":format!("{}{}",uid(),uid()),"worker_id":worker});
                    let assignment = json!({"task":spec,"lease":lease});
                    assignments.push(assignment);
                    if assignments.len() as u64 == max {
                        break;
                    }
                }
                if assignments.is_empty() {
                    return Ok(json!({"status":"empty","assignments":[]}));
                }
                let leases = assignments
                    .iter()
                    .map(|a| a["lease"].clone())
                    .collect::<Vec<_>>();
                self.commit_with_graph(
                    json!([{"type":"Leased","leases":leases}]),
                    fault,
                    Some(graph),
                )?;
                for lease in leases {
                    self.deadlines
                        .insert(field(&lease, "task_id")?.into(), now + self.lease_seconds);
                }
                Ok(json!({"status":"acquired","assignments":assignments}))
            }
            "save_task_progress_many" | "yield_tasks" => {
                let yielding = method == "yield_tasks";
                let updates = array(args, "updates")?;
                let request = self.request(
                    if yielding {
                        "yield_many"
                    } else {
                        "progress_many"
                    },
                    field(args, "request_id")?,
                    &json!(updates),
                )?;
                if let Some(response) = request.2 {
                    return Ok(response);
                }
                if updates.is_empty() {
                    return fail("ValueError", "Empty progress batch");
                }
                if yielding
                    && !crate::queue_admin::has_capacity(self.next_pending_order, updates.len())
                {
                    return fail("ValueError", "Queue scheduling order is exhausted");
                }
                let mut events = vec![];
                let mut unique = HashSet::new();
                let mut reader = Reader::new(self.store.config.clone());
                for update in updates {
                    validate_scheduling(update)?;
                    let lease = &update["lease"];
                    let id = field(lease, "task_id")?;
                    if !unique.insert(id) {
                        return fail("ValueError", "Duplicate progress task");
                    }
                    self.valid_lease(lease, now)?;
                    let validated = reader.validate(&update["input_ref"], None, None);
                    if validated
                        .as_ref()
                        .is_err_and(|e| e.kind == "StorageUnavailable")
                    {
                        self.storage_failed = true;
                    }
                    validated?;
                    let mut event = json!({"type":if yielding {"Yielded"} else {"TaskProgress"},"task_id":id,"input_ref":update["input_ref"]});
                    if yielding {
                        for key in ["priority", "scheduling_key", "metadata"] {
                            if let Some(value) = update.get(key) {
                                event[key] = value.clone();
                            }
                        }
                    }
                    events.push(event);
                }
                let last = events.last_mut().unwrap();
                last["request_key"] = json!(request.0);
                last["request_digest"] = json!(request.1);
                let response = json!(if yielding { "pending" } else { "saved" });
                last["response"] = response.clone();
                self.commit_with_graph(json!(events), fault, Some(graph))?;
                Ok(response)
            }
            "release_tasks" => {
                let leases = array(args, "leases")?;
                let request =
                    self.request("release_tasks", field(args, "request_id")?, &json!(leases))?;
                if let Some(response) = request.2 {
                    return Ok(response);
                }
                let mut events = vec![];
                let mut response = vec![];
                let mut unique = HashSet::new();
                for lease in leases {
                    if !unique.insert(field(lease, "task_id")?) {
                        return fail("ValueError", "Duplicate release task");
                    }
                    match self.valid_lease(lease, now) {
                        Ok(task) => {
                            events.push(json!({"type":"Yielded","task_id":lease["task_id"],"input_ref":task["spec"]["input_ref"]}));
                            response.push(json!("released"));
                        }
                        Err(e) if ["StaleAttempt", "LeaseExpired"].contains(&e.kind.as_str()) => {
                            response.push(json!(e.kind))
                        }
                        Err(e) => return Err(e),
                    }
                }
                if !crate::queue_admin::has_capacity(self.next_pending_order, events.len()) {
                    return fail("ValueError", "Queue scheduling order is exhausted");
                }
                if let Some(last) = events.last_mut() {
                    last["request_key"] = json!(request.0);
                    last["request_digest"] = json!(request.1);
                    last["response"] = json!(response);
                    self.commit_with_graph(json!(events), fault, Some(graph))?;
                }
                Ok(json!(response))
            }
            "heartbeat" => {
                let leases = array(args, "leases")?;
                let mut results = vec![];
                for lease in leases {
                    match self.valid_lease(lease, now) {
                        Ok(_) => {
                            self.deadlines
                                .insert(field(lease, "task_id")?.into(), now + self.lease_seconds);
                            results.push(json!("extended"));
                        }
                        Err(e) if ["StaleAttempt", "LeaseExpired"].contains(&e.kind.as_str()) => {
                            results.push(json!(e.kind))
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(json!(results))
            }
            "complete_task" => {
                let started = Instant::now();
                let lease = &args["lease"];
                let id = field(args, "submission_id")?;
                if id.is_empty() || id.len() > 1024 {
                    return fail("ValueError", "Invalid submission ID");
                }
                let reference = &args["result_ref"];
                let content = &args["result_digest"];
                let previous = self.state["submissions"][id].clone();
                if !previous.is_null() {
                    let receipt = previous["receipt"].clone();
                    if previous["lease"] != *lease
                        || receipt["digest"] != *content
                        || reference["digest"] != *content
                    {
                        return fail(
                            "IdempotencyConflict",
                            "Completion ID reused with different task/attempt/content",
                        );
                    };
                    self.count("duplicate_completions");
                    return Ok(receipt);
                }
                let task = self.valid_lease(lease, now)?;
                if reference["digest"] != *content {
                    return fail("InvalidReference", "Result digest differs from reference");
                };
                if number(reference, "records") == 0 && !boolean(&task["spec"], "allow_empty") {
                    return fail("InvalidReference", "Task does not allow empty output");
                }
                self.validate_graph(
                    reference,
                    Some(field(lease, "task_id")?),
                    Some(field(lease, "attempt_id")?),
                    graph,
                )?;
                let receipt = json!({"commit_id":uid(),"task_id":lease["task_id"],"submission_id":id,"digest":content,"position":array(&self.state,"commits")?.len(),"result_ref":reference});
                self.commit_with_graph(
                    json!([{"type":"Completed","receipt":receipt,"lease":lease}]),
                    fault,
                    Some(graph),
                )?;
                fault("before_complete_reply")?;
                self.complete_seconds.push(started.elapsed().as_secs_f64());
                if self.complete_seconds.len() > 4096 {
                    self.complete_seconds.remove(0);
                }
                Ok(receipt)
            }
            "fail_task" => {
                let lease = &args["lease"];
                let retryable = args["retryable"].as_bool().unwrap_or(true);
                let failure = &args["failure"];
                let content = json!({"lease":lease,"failure":failure,"retryable":retryable});
                let request = self.request("fail", field(args, "request_id")?, &content)?;
                if let Some(response) = request.2 {
                    return Ok(response);
                }
                let task = self.valid_lease(lease, now)?;
                let response = json!(if retryable
                    && number(&task, "failures") + 1 < number(&task["spec"], "max_attempts")
                {
                    "pending"
                } else {
                    "failed"
                });
                self.commit_with_graph(json!([Self::event("Failed",&request,response.clone(),json!({"task_ids":[lease["task_id"]],"failure":failure,"retryable":retryable}))]),fault, Some(graph))?;
                if matches!(
                    failure["category"].as_str(),
                    Some("QuotaExceeded" | "StorageUnavailable")
                ) {
                    self.storage_failed = true;
                }
                Ok(response)
            }
            "yield_task" | "save_task_progress" => {
                let content = json!({"lease":args["lease"],"input_ref":args["input_ref"]});
                let yielding = method == "yield_task";
                let request = self.request(
                    if yielding { "yield" } else { "task_progress" },
                    field(args, "request_id")?,
                    &content,
                )?;
                if let Some(response) = request.2 {
                    return Ok(response);
                }
                self.valid_lease(&args["lease"], now)?;
                if yielding && !crate::queue_admin::has_capacity(self.next_pending_order, 1) {
                    return fail("ValueError", "Queue scheduling order is exhausted");
                }
                self.validate_graph(&args["input_ref"], None, None, graph)?;
                let response = json!(if yielding { "pending" } else { "saved" });
                self.commit_with_graph(
                    json!([Self::event(
                        if yielding { "Yielded" } else { "TaskProgress" },
                        &request,
                        response.clone(),
                        json!({"task_id":args["lease"]["task_id"],"input_ref":args["input_ref"]})
                    )]),
                    fault,
                    Some(graph),
                )?;
                Ok(response)
            }
            "cancel_task" => {
                let id = field(args, "task_id")?;
                let request = self.request("cancel", field(args, "request_id")?, &json!(id))?;
                if let Some(response) = request.2 {
                    return Ok(response);
                }
                if terminal(&self.state["tasks"][id]["state"]) {
                    return fail("ValueError", "Cannot cancel terminal task");
                };
                self.commit_with_graph(
                    json!([Self::event(
                        "Cancelled",
                        &request,
                        json!("cancelled"),
                        json!({"task_id":id})
                    )]),
                    fault,
                    Some(graph),
                )?;
                Ok(json!("cancelled"))
            }
            "seal_input" => {
                let request = self.request("seal", field(args, "request_id")?, &Value::Null)?;
                if request.2.is_none() {
                    self.commit_with_graph(
                        json!([Self::event("Sealed", &request, json!("sealed"), json!({}))]),
                        fault,
                        Some(graph),
                    )?;
                }
                Ok(json!("sealed"))
            }
            "lookup_submission" => {
                Ok(self.state["submissions"][field(args, "submission_id")?]["receipt"].clone())
            }
            "pending_tasks" => {
                let prefix = args["task_prefix"].as_str();
                let minimum = args["min_priority"].as_i64();
                Ok(json!(
                    self.pending
                        .values()
                        .filter_map(|id| {
                            let spec = &self.state["tasks"][id]["spec"];
                            if prefix.is_some_and(|p| !id.starts_with(p))
                                || minimum
                                    .is_some_and(|p| spec["priority"].as_i64().unwrap_or(0) < p)
                            {
                                None
                            } else {
                                Some(spec.clone())
                            }
                        })
                        .collect::<Vec<_>>()
                ))
            }
            "task_status" => Ok(self.state["tasks"][field(args, "task_id")?].clone()),
            "producer_state" => Ok(self.state["producers"][field(args, "producer_id")?].clone()),
            "read_commits" => {
                let commits = array(&self.state, "commits")?;
                let cursor = number(args, "cursor") as usize;
                let limit = args["limit"].as_u64().unwrap_or(100) as usize;
                if cursor > commits.len() || limit == 0 {
                    return fail("ValueError", "Invalid cursor/page limit");
                }
                let end = cursor.saturating_add(limit).min(commits.len());
                if cursor < number(&self.state, "retired_prefix") as usize
                    || array(&self.state, "retired_positions")?
                        .iter()
                        .filter_map(Value::as_u64)
                        .any(|p| (cursor..end).contains(&(p as usize)))
                {
                    return fail(
                        "InvalidReference",
                        "Requested history was released; resume from a retained checkpoint or a live cursor",
                    );
                }
                Ok(
                    json!({"commits":&commits[cursor..end],"cursor":end,"end_of_input":self.sealed&&end==commits.len()&&vals(&self.state["tasks"]).all(|t|terminal(&t["state"]))}),
                )
            }
            "retire_worker" => {
                let worker = field(args, "worker_id")?;
                let leases = vals(&self.state["tasks"])
                    .filter(|t| t["state"] == "leased" && t["lease"]["worker_id"] == worker)
                    .map(|t| t["lease"].clone())
                    .collect::<Vec<_>>();
                for lease in leases {
                    let call = json!({"lease":lease,"request_id":format!("retire:{}",field(&lease,"attempt_id")?),"failure":{"category":"WorkerLost"},"retryable":true});
                    match self.call("fail_task", &call, now, fault) {
                        Ok(_) => {}
                        Err(e) if e.kind == "LeaseExpired" => {
                            self.commit_with_graph(
                                json!([{"type":"Expired","task_ids":[lease["task_id"]]}]),
                                fault,
                                Some(graph),
                            )?;
                        }
                        Err(e) => return Err(e),
                    }
                }
                self.commit_with_graph(
                    json!([{"type":"ReaderRetired","worker_id":worker}]),
                    fault,
                    Some(graph),
                )?;
                Ok(json!(array(&self.state,"commits")?.iter().filter(|r|self.state["tasks"][r["task_id"].as_str().unwrap_or("")]["lease"]["worker_id"]==worker).cloned().collect::<Vec<_>>()))
            }
            "open_consumer" => {
                let id = field(args, "consumer_id")?;
                if id.is_empty()
                    || id.len() > 1024
                    || args["exclusive_owner"]
                        .as_str()
                        .is_none_or(|s| s.is_empty())
                    || self.owners.contains_key(id)
                {
                    return fail(
                        "UnsafeRecovery",
                        "Consumer requires exclusive single-builder ownership",
                    );
                };
                let token = format!("{}{}", uid(), uid());
                self.owners.insert(id.into(), token.clone());
                Ok(json!(token))
            }
            "close_consumer" => {
                let id = field(args, "consumer_id")?;
                self.consumer(id, field(args, "token")?)?;
                self.owners.remove(id);
                Ok(Value::Null)
            }
            "load_consumer_state" => {
                Ok(self.state["consumers"][field(args, "consumer_id")?].clone())
            }
            "save_consumer_state" => {
                let id = field(args, "consumer_id")?;
                self.consumer(id, field(args, "token")?)?;
                let state = self.consumer_state(args)?;
                let request = self.request(
                    &format!("consumer:{id}"),
                    field(args, "request_id")?,
                    &state,
                )?;
                if let Some(response) = request.2 {
                    return Ok(response);
                };
                self.validate_consumer_state(&state, graph)?;
                self.commit_with_graph(
                    json!([Self::event(
                        "ConsumerState",
                        &request,
                        state.clone(),
                        json!({"consumer_id":id,"state":state})
                    )]),
                    fault,
                    Some(graph),
                )?;
                Ok(state)
            }
            "plan_batch" => {
                let id = field(args, "consumer_id")?;
                self.consumer(id, field(args, "token")?)?;
                let batch_id = field(args, "batch_id")?;
                let positions = array(args, "input_positions")?;
                let unique: HashSet<_> = positions.iter().filter_map(|p| p.as_u64()).collect();
                if batch_id.is_empty() || batch_id.len() > 1024 || unique.len() != positions.len() {
                    return fail("ValueError", "Invalid batch ID/duplicate inputs");
                };
                if unique
                    .iter()
                    .any(|p| *p >= self.state["commits"].as_array().unwrap().len() as u64)
                {
                    return fail("InvalidReference", "Batch plan refers to unaccepted input");
                }
                let batch = json!({"batch_id":batch_id,"consumer_id":id,"input_positions":positions,"plan_ref":args["plan_ref"],"ready":false});
                let request = self.request("plan_batch", batch_id, &batch)?;
                if request.2.is_some() {
                    return Ok(self.state["batches"][batch_id].clone());
                };
                self.validate_graph(&args["plan_ref"], None, None, graph)?;
                self.commit_with_graph(
                    json!([Self::event(
                        "BatchPlanned",
                        &request,
                        batch.clone(),
                        json!({"batch_id":batch_id,"batch":batch})
                    )]),
                    fault,
                    Some(graph),
                )?;
                Ok(batch)
            }
            "batch_ready" => {
                let id = field(args, "consumer_id")?;
                self.consumer(id, field(args, "token")?)?;
                let batch = field(args, "batch_id")?;
                if self.state["batches"][batch]["consumer_id"] != id {
                    return fail("InvalidReference", "Batch belongs to another consumer");
                };
                let state = self.consumer_state(args)?;
                let content = json!({"batch_id":batch,"ready_ref":args["ready_ref"],"state":state});
                let request = self.request("batch_ready", batch, &content)?;
                if let Some(response) = request.2 {
                    return Ok(response);
                };
                self.validate_graph(&args["ready_ref"], None, None, graph)?;
                self.validate_consumer_state(&state, graph)?;
                self.commit_with_graph(json!([Self::event("BatchReady",&request,content.clone(),json!({"consumer_id":id,"batch_id":batch,"ready_ref":args["ready_ref"],"state":state}))]),fault, Some(graph))?;
                Ok(content)
            }
            "get_batch" => Ok(self.state["batches"][field(args, "batch_id")?].clone()),
            "register_checkpoint" => {
                let id = field(args, "checkpoint_id")?;
                let reference = &args["checkpoint_ref"];
                // The original checkpoint content is already durable in its
                // receipt. A released checkpoint may have been physically
                // collected; retry must neither reread nor register it again.
                let key = Self::request_key("checkpoint", id)?;
                if let Some(content) =
                    crate::request::response(self.state["requests"].as_object().unwrap(), &key)
                {
                    if content["checkpoint_ref"] != *reference {
                        return fail(
                            "IdempotencyConflict",
                            "Checkpoint ID reused with a different reference",
                        );
                    }
                    if let Some(response) = self.request("checkpoint", id, content)?.2 {
                        return Ok(response);
                    }
                    return fail("UnsafeRecovery", "Checkpoint receipt disappeared");
                }
                let members = self.validate_graph(reference, None, None, graph)?;
                if members.len() != 1 {
                    return fail("InvalidReference", "Expected one JSON checkpoint record");
                };
                let record = self.store.config.read(&members[0])?;
                if record.metadata["codec"] != "json.v1" {
                    return fail("InvalidReference", "Expected JSON checkpoint");
                };
                let checkpoint: Value = serde_json::from_slice(&record.payload)?;
                if checkpoint["version"] != 1 || checkpoint["checkpoint_id"] != id {
                    return fail("InvalidReference", "Checkpoint identity/schema mismatch");
                };
                self.validate_graph(&checkpoint["consumer_state"], None, None, graph)?;
                if array(&checkpoint, "batch_ids")?
                    .iter()
                    .any(|id| !boolean(&self.state["batches"][id.as_str().unwrap_or("")], "ready"))
                {
                    return fail(
                        "InvalidReference",
                        "Checkpoint contains unknown/unready batch",
                    );
                }
                let root = self.store.config.manifest(reference)?;
                let dependencies = array(&root, "dependencies")?;
                let mut required = array(&checkpoint, "training_dependencies")?.clone();
                required.push(checkpoint["consumer_state"].clone());
                if required.iter().any(|r| !dependencies.contains(r)) {
                    return fail(
                        "InvalidReference",
                        "Checkpoint dependencies are not committed under root",
                    );
                }
                let mut content = checkpoint;
                content["checkpoint_ref"] = reference.clone();
                let request = self.request("checkpoint", id, &content)?;
                if request.2.is_none() {
                    self.commit_with_graph(
                        json!([Self::event(
                            "Checkpoint",
                            &request,
                            content.clone(),
                            json!({"checkpoint_id":id,"checkpoint":content})
                        )]),
                        fault,
                        Some(graph),
                    )?;
                }
                Ok(content)
            }
            "snapshot" => {
                let mut state = self.state.clone();
                state["version"] = json!(1);
                state["journal_sequence"] = json!(self.journal.sequence - 1);
                state["journal_bytes"] = json!(self.journal.size());
                state["sealed"] = json!(self.sealed);
                state["epoch"] = json!(self.epoch);
                let id = format!("snapshot:{}", self.journal.sequence);
                let record = Record {
                    metadata: json!({"record_id":id,"codec":"json.v1","metadata":{},"tokens":0}),
                    payload: bytes(&state)?.into(),
                };
                Ok(self.store.publish(
                    &json!([{"records":[0],"dependencies":[]}]),
                    vec![record],
                    &id,
                    fault,
                )?[0]
                    .clone())
            }
            "metrics" => {
                let mut states = json!({});
                for task in vals(&self.state["tasks"]) {
                    let name = field(task, "state")?;
                    states[name] = json!(number(&states, name) + 1)
                }
                let usage = self.usage()?;
                Ok(
                    json!({"tasks":states,"usage_with_reservations":usage,"counters":self.counters,"journal_bytes":self.journal.size(),"recovery_seconds":self.recovery_seconds,"complete_seconds":self.complete_seconds,"epoch":self.epoch}),
                )
            }
            "drain" => {
                self.draining = true;
                Ok(json!(
                    vals(&self.state["tasks"])
                        .filter(|t| t["state"] == "leased")
                        .count()
                ))
            }
            "usage" => self.usage(),
            "state" => {
                let mut state = self.state.clone();
                state["epoch"] = json!(self.epoch);
                state["sealed"] = json!(self.sealed);
                state["draining"] = json!(self.draining);
                state["storage_failed"] = json!(self.storage_failed);
                state["journal_sequence"] = json!(self.journal.sequence);
                state["counters"] = self.counters.clone();
                Ok(state)
            }
            _ => fail("ValueError", format!("Unknown queue operation {method}")),
        }
    }
    fn training_roots(
        &self,
        state: &Value,
        progress: &Value,
        roots: &mut HashMap<String, Value>,
    ) -> Result<()> {
        crate::queue_roots::training(&mut RootDigest, &self.state, state, progress, roots)
            .map_err(root_error)
    }
    fn collect_garbage(&mut self, fault: &mut Fault<'_>) -> Result<Value> {
        if !self.store.config.gc_enabled()? {
            return fail("ValueError", "Online GC is not enabled");
        }
        // Root selection, restricted preflight, durable retention and owner
        // naming share the same exclusively borrowed queue and journal state.
        let prepared = self
            .journal
            .prepare_gc(
                &mut self.state,
                &mut self.progress_cache,
                &mut RootDigest,
                &self.storage_prefix,
                GcPreflight {
                    config: &self.store.config,
                    prefix: &self.storage_prefix,
                    pending: &mut self.pending_consumption,
                    storage_failed: &mut self.storage_failed,
                    fault,
                },
            )
            .map_err(gc_prepare_error)?;
        self.store
            .config
            .collect_queue(prepared, fault)?
            .ok_or_else(|| Error::new("ValueError", "Online GC is not enabled"))
    }
    pub fn close(&mut self) {
        self.draining = true;
        self.journal.close();
    }
}

fn handoff_error(error: crate::queue_handoff::Error<Error>) -> Error {
    use crate::queue_handoff::Error as Handoff;
    match error {
        Handoff::Digest(error) => error,
        other => Error::new(
            "CorruptData",
            match other {
                Handoff::Transaction => "Invalid transaction",
                Handoff::Misplaced => "Misplaced staging handoff marker",
                Handoff::Repeated => "Repeated staging handoff marker",
                Handoff::Identity => "Staging handoff identity mismatch",
                Handoff::Empty => "Staging handoff has no roots",
                Handoff::Digest(_) => unreachable!(),
            },
        ),
    }
}

#[cfg(test)]
fn recovered_consumption(
    prefix: &str,
    sequence: u64,
    transaction: &Value,
) -> Result<Option<(String, Vec<Value>)>> {
    crate::queue_handoff::recover(&mut RootDigest, prefix, sequence, transaction)
        .map_err(handoff_error)
}

struct ProgressReader<'a> {
    config: &'a Config,
    validated_members: Option<&'a [Value]>,
    storage_failed: &'a mut bool,
}
impl crate::queue_progress::Reader for ProgressReader<'_> {
    type Error = Error;
    fn members(&mut self, reference: &Value) -> Result<Vec<Value>> {
        let result = match self.validated_members {
            Some(members) => Ok(members.to_vec()),
            None => self.config.validate(reference, None, None),
        };
        if result
            .as_ref()
            .is_err_and(|error| error.kind == "StorageUnavailable")
        {
            *self.storage_failed = true;
        }
        result
    }
    fn read(&mut self, reference: &Value) -> Result<crate::queue_progress::Record> {
        let record = self.config.read(reference)?;
        Ok(crate::queue_progress::Record {
            metadata: record.metadata,
            payload: record.payload,
        })
    }
}
// This adapter cannot borrow queue business state or its WAL. The checked
// preparation driver keeps both exclusively borrowed while this callback runs.
struct GcPreflight<'a, 'f> {
    config: &'a Config,
    prefix: &'a str,
    pending: &'a mut Vec<(String, Vec<Value>)>,
    storage_failed: &'a mut bool,
    fault: &'a mut Fault<'f>,
}
impl crate::queue_gc_prepare::Preflight<Error> for GcPreflight<'_, '_> {
    fn prepare(&mut self) -> Result<()> {
        finish_consumption(self.config, self.prefix, self.pending, self.fault)?;
        let retained = (|| {
            let verified = self.config.verify_reactivation(&[], self.fault)?;
            self.config.catalog(|catalog| {
                catalog.retain_verified(self.config, &verified, self.fault)?;
                catalog.retain_many(self.config, &[], self.fault)
            })
        })();
        if retained
            .as_ref()
            .is_err_and(|error| error.kind == "StorageUnavailable")
        {
            *self.storage_failed = true;
        }
        retained.map(|_| ())
    }
}
impl<'n> crate::wal_sink::Hooks<crate::wal_file::NativeFile<'n>> for GcPreflight<'_, '_> {
    fn point(&mut self, _file: &mut crate::wal_file::NativeFile<'n>, point: &str) -> Result<()> {
        (self.fault)(point)
    }
}
impl crate::queue_progress::Reader for GcPreflight<'_, '_> {
    type Error = Error;
    fn members(&mut self, reference: &Value) -> Result<Vec<Value>> {
        crate::queue_progress::Reader::members(
            &mut ProgressReader {
                config: self.config,
                validated_members: None,
                storage_failed: self.storage_failed,
            },
            reference,
        )
    }
    fn read(&mut self, reference: &Value) -> Result<crate::queue_progress::Record> {
        crate::queue_progress::Reader::read(
            &mut ProgressReader {
                config: self.config,
                validated_members: None,
                storage_failed: self.storage_failed,
            },
            reference,
        )
    }
}
fn finish_consumption(
    config: &Config,
    prefix: &str,
    pending: &mut Vec<(String, Vec<Value>)>,
    fault: &mut Fault<'_>,
) -> Result<()> {
    finish_consumption_with_graph(config, prefix, pending, fault, None)
}

fn finish_consumption_with_graph(
    config: &Config,
    prefix: &str,
    pending: &mut Vec<(String, Vec<Value>)>,
    fault: &mut Fault<'_>,
    graph: Option<&GraphProof>,
) -> Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    fault("before_committed_staging_cleanup")?;
    // One refresh/lock for historical receipt checks; successful receipts
    // do not reread manifests, append, or consume subsequently restaged roots.
    config
        .catalog(|catalog| {
            while let Some((id, roots)) = pending.last() {
                if !catalog.handoff_consumed(id) {
                    let owners = roots
                        .iter()
                        .map(|root| Ok(format!("{prefix}{}", digest(root)?)))
                        .collect::<Result<HashSet<_>>>()?;
                    catalog.require_retained(&owners)?;
                }
                let outcome = catalog.consume_once_with_graph(config, id, roots, graph, fault);
                crate::consumption::pop_completed(pending, outcome.is_ok());
                outcome?;
            }
            Ok(())
        })?
        .ok_or_else(|| {
            Error::new(
                "UnsafeRecovery",
                "Committed staging cleanup requires its storage catalog",
            )
        })?;
    fault("after_committed_staging_cleanup")
}

struct RootDigest;
impl crate::closure_node::Digest for RootDigest {
    type Error = Error;
    fn digest(&mut self, value: &Value) -> Result<String> {
        digest(value)
    }
}
fn root_error(error: crate::queue_roots::Error<Error>) -> Error {
    match error {
        crate::queue_roots::Error::Array(name) => {
            Error::new("InvalidReference", format!("Missing array {name}"))
        }
        crate::queue_roots::Error::Digest(error) => error,
    }
}
fn progress_error(error: crate::queue_progress::Error<Error>) -> Error {
    match error {
        crate::queue_progress::Error::Read(error) => error,
        crate::queue_progress::Error::Json(error) => error.into(),
        crate::queue_progress::Error::Members => {
            Error::new("InvalidReference", "Expected one JSON progress record")
        }
        crate::queue_progress::Error::Codec => {
            Error::new("InvalidReference", "Expected JSON progress")
        }
        crate::queue_progress::Error::Array(name) => {
            Error::new("InvalidReference", format!("Missing array {name}"))
        }
        crate::queue_progress::Error::Invalid => {
            Error::new("InvalidReference", "Invalid sparse consumer progress")
        }
        crate::queue_progress::Error::Duplicate => {
            Error::new("InvalidReference", "Duplicate progress entries")
        }
    }
}
fn gc_prepare_error(error: crate::queue_gc_prepare::Error<Error, Error>) -> Error {
    match error {
        crate::queue_gc_prepare::Error::Roots(error) => root_error(error),
        crate::queue_gc_prepare::Error::Progress(error) => progress_error(error),
        crate::queue_gc_prepare::Error::Preflight(error) => error,
        crate::queue_gc_prepare::Error::Closed => Error::new(
            "UnsafeRecovery",
            "Journal unavailable; stop owner and recover",
        ),
        crate::queue_gc_prepare::Error::Retire(error) => crate::journal::retirement_error(error),
    }
}
fn references(value: &Value, result: &mut HashMap<String, Value>) -> Result<()> {
    crate::queue_roots::references(&mut RootDigest, value, result)
}

#[cfg(test)]
mod consumption_tests {
    use super::*;

    fn transaction() -> Result<Value> {
        let mut events = json!([{"type":"Submitted","tasks":[{
            "input_ref":{"manifest":{"segment":{"path":"data"}},"digest":"root"}
        }]}]);
        let id = digest(&json!(["queue:test:", 7, events]))?;
        events
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"StagingHandoff","id":id}));
        Ok(events)
    }

    #[test]
    fn handoff_identity_binds_queue_sequence_and_business_content() -> Result<()> {
        let mut events = transaction()?;
        let (_, roots) = recovered_consumption("queue:test:", 7, &events)?.unwrap();
        assert_eq!(roots.len(), 1);
        assert!(recovered_consumption("queue:other:", 7, &events).is_err());
        assert!(recovered_consumption("queue:test:", 8, &events).is_err());
        events[0]["tasks"][0]["input_ref"]["digest"] = json!("different");
        assert!(recovered_consumption("queue:test:", 7, &events).is_err());
        Ok(())
    }

    #[test]
    fn serialized_handoff_recovers_exact_identity_with_signed_zero_and_nested_numbers() -> Result<()>
    {
        let mut events = transaction()?;
        events.as_array_mut().unwrap().pop();
        events[0]["metadata"] =
            json!({"unicode":"续训", "numbers":[-0.0,0.0,0.1,u64::MAX,i64::MIN,1.0e-200]});
        let roots = crate::queue_handoff::roots(&mut RootDigest, events.as_array().unwrap())?;
        let attached = crate::queue_handoff::attach(
            &mut RootDigest,
            "queue:test:",
            7,
            events.as_array_mut().unwrap(),
            roots.into_values().collect(),
        )?
        .unwrap();
        let raw = serde_json::to_vec(&events)?;
        let decoded: Value = serde_json::from_slice(&raw)?;
        assert_eq!(serde_json::to_vec(&decoded)?, raw);
        let recovered = recovered_consumption("queue:test:", 7, &decoded)?.unwrap();
        assert_eq!(recovered.0, attached.0);
        assert_eq!(recovered.1, attached.1);
        let mut corrupted = decoded;
        corrupted[0]["metadata"]["numbers"][0] = json!(0.0);
        assert!(recovered_consumption("queue:test:", 7, &corrupted).is_err());
        Ok(())
    }

    #[test]
    fn handoff_marker_must_be_unique_last_and_have_roots() -> Result<()> {
        let mut misplaced = transaction()?;
        misplaced.as_array_mut().unwrap().reverse();
        assert!(recovered_consumption("queue:test:", 7, &misplaced).is_err());
        let mut repeated = transaction()?;
        let marker = repeated[1].clone();
        repeated.as_array_mut().unwrap().push(marker);
        assert!(recovered_consumption("queue:test:", 7, &repeated).is_err());
        let business = json!([{"type":"Epoch","epoch":"e","revoked":[]}]);
        let id = digest(&json!(["queue:test:", 7, business]))?;
        let no_roots = json!([business[0],{"type":"StagingHandoff","id":id}]);
        assert!(recovered_consumption("queue:test:", 7, &no_roots).is_err());
        Ok(())
    }

    #[test]
    fn legacy_history_does_not_authorize_consuming_later_staging() -> Result<()> {
        let mut events = transaction()?;
        events.as_array_mut().unwrap().pop();
        assert!(recovered_consumption("queue:test:", 7, &events)?.is_none());
        Ok(())
    }

    #[test]
    fn gc_preflight_failure_preserves_selected_roots_and_journal() -> Result<()> {
        let root = tempfile::tempdir()?;
        let mut config = Config::new(
            root.path().to_str().unwrap(),
            "gc-preflight",
            vec!["bytes.v1".into()],
            1024 * 1024,
            4 * 1024 * 1024,
            0,
        )?;
        config.online_gc = true;
        let mut queue = Coordinator::open(
            config,
            &json!({"exclusive_owner":"native gc preflight test"}),
            &mut |_| Ok(()),
        )?;
        let continuation = queue.store.publish(
            &json!([{"records":[0],"dependencies":[]}]),
            vec![Record {
                metadata: json!({"record_id":"cursor","codec":"bytes.v1","metadata":{},"tokens":0}),
                payload: b"cursor".to_vec().into(),
            }],
            "gc-preflight-state",
            &mut |_| Ok(()),
        )?[0]
            .clone();
        let error = queue.call(
            "submit_tasks",
            &json!({"request_id":"producer","tasks":[],"producer_id":"sampler","producer_state":{"state_ref":continuation}}),
            0., &mut |at| if at == "before_staging_consume" {
                Err(Error::new("Injected", "accepted but pending consumption"))
            } else { Ok(()) },
        ).unwrap_err();
        assert_eq!(error.kind, "Injected");
        assert!(!queue.pending_consumption.is_empty());
        let before = queue.state.clone();
        let sequence = queue.journal.sequence;
        let journal = std::fs::read(&queue.journal.path)?;
        let catalog = std::fs::read(root.path().join("storage.log"))?;
        let mut points = Vec::new();
        let error = queue
            .collect_garbage(&mut |at| {
                points.push(at.to_owned());
                if at == "before_committed_staging_cleanup" {
                    Err(Error::new("Injected", "preflight failed"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert_eq!(error.message, "preflight failed");
        assert_eq!(points, vec!["before_committed_staging_cleanup"]);
        assert_eq!(queue.state, before);
        assert_eq!(queue.journal.sequence, sequence);
        assert_eq!(std::fs::read(&queue.journal.path)?, journal);
        assert_eq!(std::fs::read(root.path().join("storage.log"))?, catalog);
        queue.collect_garbage(&mut |_| Ok(()))?;
        assert!(queue.pending_consumption.is_empty());
        assert_eq!(queue.state["producers"], before["producers"]);
        assert!(
            root.path()
                .join(field(&continuation["manifest"]["segment"], "path")?)
                .exists()
        );
        queue.close();
        Ok(())
    }

    #[test]
    fn order_capacity_rejects_new_writes_before_wal_but_keeps_cached_replies() -> Result<()> {
        let root = tempfile::tempdir()?;
        let config = Config::new(
            root.path().to_str().unwrap(),
            "capacity",
            vec!["bytes.v1".into()],
            1024 * 1024,
            4 * 1024 * 1024,
            1024 * 1024,
        )?;
        let mut queue = Coordinator::open(
            config,
            &json!({"exclusive_owner":"native capacity test"}),
            &mut |_| Ok(()),
        )?;
        let task = json!({"task_id":"a","input_ref":null,"max_attempts":3,
            "estimated_records":1,"estimated_bytes":0,"estimated_tokens":0,"control":false});
        let request = json!({"request_id":"accepted","tasks":[task]});
        assert_eq!(
            queue.call("submit_tasks", &request, 0., &mut |_| Ok(()))?,
            json!(["a"])
        );
        // Exercise the otherwise impractically distant resource boundary.
        queue.next_pending_order = usize::MAX;
        let sequence = queue.journal.sequence;
        let before = queue.state.clone();
        assert_eq!(
            queue.call("submit_tasks", &request, 1., &mut |_| panic!(
                "cached reply must not write"
            ))?,
            json!(["a"])
        );
        let mut fresh = request["tasks"][0].clone();
        fresh["task_id"] = json!("b");
        let error = queue
            .call(
                "submit_tasks",
                &json!({"request_id":"new","tasks":[fresh]}),
                1.,
                &mut |_| panic!("capacity rejection must precede writes"),
            )
            .unwrap_err();
        assert_eq!(error.kind, "ValueError");
        assert_eq!(error.message, "Queue scheduling order is exhausted");
        assert_eq!(queue.state, before);
        assert_eq!(queue.journal.sequence, sequence);
        assert!(!queue.journal.poisoned);

        let assignment = queue.call("acquire", &json!({"worker_id":"worker"}), 2., &mut |_| {
            Ok(())
        })?;
        let lease = assignment["assignments"][0]["lease"].clone();
        let input = queue.store.publish(
            &json!([{"records":[0],"dependencies":[]}]),
            vec![Record {
                metadata: json!({"record_id":"continuation","codec":"bytes.v1","metadata":{},"tokens":0}),
                payload: b"continuation".to_vec().into(),
            }],
            "capacity-input",
            &mut |_| Ok(()),
        )?[0].clone();
        let sequence = queue.journal.sequence;
        let before = queue.state.clone();
        let error = queue
            .call(
                "yield_task",
                &json!({"request_id":"yield","lease":lease,"input_ref":input}),
                3.,
                &mut |_| panic!("capacity rejection must precede writes"),
            )
            .unwrap_err();
        assert_eq!(error.kind, "ValueError");
        assert_eq!(error.message, "Queue scheduling order is exhausted");
        assert_eq!(queue.state, before);
        assert_eq!(queue.journal.sequence, sequence);
        assert!(!queue.journal.poisoned);
        for (method, request) in [
            (
                "yield_tasks",
                json!({"request_id":"yield-many", "updates":[{"lease":lease,"input_ref":input}]}),
            ),
            (
                "release_tasks",
                json!({"request_id":"release-many", "leases":[lease]}),
            ),
        ] {
            let error = queue
                .call(method, &request, 3., &mut |_| {
                    panic!("batch capacity rejection must precede writes")
                })
                .unwrap_err();
            assert_eq!(error.kind, "ValueError");
            assert_eq!(error.message, "Queue scheduling order is exhausted");
            assert_eq!(queue.state, before);
            assert_eq!(queue.journal.sequence, sequence);
            assert!(!queue.journal.poisoned);
        }
        queue.close();
        Ok(())
    }
}
