//! Single-owner durable scheduling and accepted-result state machine.
use crate::journal::Journal;
use crate::store::{Config, Reader, Record, Store, mkdir, sync, sync_dir};
use crate::{Error, Fault, Result, array, bytes, digest, fail, field, number, uid};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::OpenOptions,
    io::Write,
    time::Instant,
};

pub struct Coordinator {
    pub store: Store,
    pub journal: Journal,
    pub state: Value,
    pub limits: Value,
    pub epoch: String,
    pub recovery_seconds: f64,
    order: Vec<String>,
    positions: HashMap<String, usize>,
    pending: BTreeMap<usize, String>,
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
    progress_cache: Option<(Value, Value)>,
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
fn retry(task: &mut Value) {
    let failures = number(task, "failures") + 1;
    task["failures"] = json!(failures);
    task["state"] = json!(if failures < number(&task["spec"], "max_attempts") {
        "pending"
    } else {
        "failed"
    });
    task["lease"] = Value::Null;
}
fn bounded(value: &Value, limit: u64) -> Result<()> {
    if bytes(value)?.len() as u64 > limit {
        return fail(
            "ResourceLimitExceeded",
            "Control metadata limit exceeded; use a manifest",
        );
    }
    Ok(())
}
impl Coordinator {
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
            state: json!({"tasks":{},"requests":{},"submissions":{},"consumers":{},"batches":{},"checkpoints":{},"commits":[],"producers":{},"retired_prefix":0,"retired_positions":[],"readers":{}}),
            limits,
            epoch: String::new(),
            recovery_seconds: 0.,
            order: vec![],
            positions: HashMap::new(),
            pending: BTreeMap::new(),
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
            progress_cache: None,
        };
        for transaction in result.journal.transactions.clone().iter().skip(1) {
            for event in transaction
                .as_array()
                .ok_or_else(|| Error::new("CorruptData", "Invalid transaction"))?
            {
                result.apply(event)?;
            }
        }
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
    fn commit(&mut self, mut events: Value, fault: &mut Fault<'_>) -> Result<()> {
        let mut refs = HashMap::new();
        let mut reactivated = HashMap::new();
        if self.store.config.gc_enabled() {
            references(&events, &mut refs)?;
            // Consumer restore can make retired receipts/batches live again.
            // Their roots are implicit in the cursor, not in the event refs.
            let training = events.as_array().unwrap().iter().find(|e| {
                matches!(e["type"].as_str(), Some("ConsumerState" | "BatchReady"))
                    && e["consumer_id"] == "training"
            });
            if let Some(event) = training {
                let state = event["state"].clone();
                let progress = self.progress(&state["progress_ref"])?;
                self.training_roots(&state, &progress, &mut reactivated)?;
                let prefix = number(&state, "processed_cursor");
                let positions = array(&progress, "processed_positions")?;
                if prefix < number(&self.state, "retired_prefix")
                    || array(&self.state, "retired_positions")?
                        .iter()
                        .any(|p| p.as_u64().is_some_and(|p| p >= prefix) && !positions.contains(p))
                {
                    events.as_array_mut().unwrap().push(json!({
                        "type":"RetentionAdvanced", "prefix":prefix,
                        "positions":positions
                    }));
                }
            }
        }
        let roots = refs
            .iter()
            .map(|(key, r)| (format!("{}{key}", self.storage_prefix), vec![r.clone()]))
            .collect::<Vec<_>>();
        let reactivated = reactivated
            .into_iter()
            .map(|(key, r)| (format!("{}{key}", self.storage_prefix), r))
            .collect::<Vec<_>>();
        let retained = self.store.config.catalog(|c| {
            c.retain_reactivated(&self.store.config, &reactivated, fault)?;
            c.retain_many(&self.store.config, &roots, fault)
        });
        if retained
            .as_ref()
            .is_err_and(|e| e.kind == "StorageUnavailable")
        {
            self.storage_failed = true;
        }
        retained?;
        self.journal.append(&events, fault)?;
        for event in events.as_array().unwrap() {
            if let Err(e) = self.apply(event) {
                self.journal.poisoned = true;
                return Err(e);
            }
        }
        self.store.config.catalog(|c| {
            c.consume(
                &self.store.config,
                &refs.into_values().collect::<Vec<_>>(),
                fault,
            )
        })?;
        Ok(())
    }
    fn apply(&mut self, e: &Value) -> Result<()> {
        let kind = field(e, "type")?;
        let affected: Vec<String> = match kind {
            "Submitted" => array(e, "tasks")?
                .iter()
                .map(|t| field(t, "task_id").map(str::to_owned))
                .collect::<Result<_>>()?,
            "Leased" => array(e, "leases")?
                .iter()
                .map(|t| field(t, "task_id").map(str::to_owned))
                .collect::<Result<_>>()?,
            "Epoch" => array(e, "revoked")?
                .iter()
                .map(|id| field_value(id).map(str::to_owned))
                .collect::<Result<_>>()?,
            "Expired" | "Failed" => array(e, "task_ids")?
                .iter()
                .map(|id| field_value(id).map(str::to_owned))
                .collect::<Result<_>>()?,
            "Cancelled" | "Yielded" | "TaskProgress" => vec![field(e, "task_id")?.into()],
            "Completed" => vec![field(&e["receipt"], "task_id")?.into()],
            _ => vec![],
        };
        for id in &affected {
            if let Some(position) = self.positions.get(id) {
                self.pending.remove(position);
            }
            self.leased.remove(id);
            if let Some(usage) = self.usage_cache.as_mut() {
                Self::adjust_active(usage, &self.state["tasks"][id], false);
            }
        }
        match kind {
            "Epoch" => {
                self.epoch = field(e, "epoch")?.into();
                for id in array(e, "revoked")? {
                    retry(&mut self.state["tasks"][field_value(id)?]);
                }
            }
            "Submitted" => {
                for spec in array(e, "tasks")? {
                    let id = field(spec, "task_id")?;
                    self.positions.insert(id.into(), self.order.len());
                    self.order.push(id.into());
                    self.state["tasks"][id] = json!({"spec":spec,"state":"pending","generation":0,"failures":0,"lease":null});
                }
                if let Some(id) = e["producer_id"].as_str() {
                    self.state["producers"][id] = e["producer_state"].clone();
                }
            }
            "Leased" => {
                for lease in array(e, "leases")? {
                    let t = &mut self.state["tasks"][field(lease, "task_id")?];
                    t["state"] = json!("leased");
                    t["generation"] = lease["generation"].clone();
                    t["lease"] = lease.clone();
                    let input = t["spec"]["input_ref"].clone();
                    if !input.is_null() {
                        self.state["readers"][field(lease, "attempt_id")?] =
                            json!({"lease":lease,"roots":[input]});
                    }
                }
            }
            "Expired" | "Failed" => {
                for id in array(e, "task_ids")? {
                    let id = field_value(id)?;
                    if kind == "Failed"
                        && let Some(attempt) = self.state["tasks"][id]["lease"]["attempt_id"]
                            .as_str()
                            .map(str::to_owned)
                    {
                        self.state["readers"]
                            .as_object_mut()
                            .unwrap()
                            .remove(&attempt);
                    }
                    let t = &mut self.state["tasks"][id];
                    if e["retryable"].as_bool().unwrap_or(true) {
                        retry(t)
                    } else {
                        t["state"] = json!("failed")
                    };
                    t["failure"] = e
                        .get("failure")
                        .cloned()
                        .unwrap_or_else(|| json!({"reason":e["type"]}));
                    self.deadlines.remove(id);
                }
            }
            "Cancelled" => {
                let id = field(e, "task_id")?;
                self.state["tasks"][id]["state"] = json!("cancelled");
                self.deadlines.remove(id);
            }
            "Yielded" | "TaskProgress" => {
                let id = field(e, "task_id")?;
                let lease = self.state["tasks"][id]["lease"].clone();
                if let Some(attempt) = lease["attempt_id"].as_str() {
                    if kind == "Yielded" {
                        self.state["readers"]
                            .as_object_mut()
                            .unwrap()
                            .remove(attempt);
                    } else if !e["input_ref"].is_null() {
                        if self.state["readers"][attempt].is_null() {
                            self.state["readers"][attempt] = json!({"lease":lease,"roots":[]});
                        }
                        let roots = self.state["readers"][attempt]["roots"]
                            .as_array_mut()
                            .unwrap();
                        if !roots.contains(&e["input_ref"]) {
                            roots.push(e["input_ref"].clone());
                        }
                    }
                }
                let t = &mut self.state["tasks"][id];
                t["spec"]["input_ref"] = e["input_ref"].clone();
                if e["type"] == "Yielded" {
                    t["state"] = json!("pending");
                    t["lease"] = Value::Null;
                    self.deadlines.remove(id);
                }
            }
            "Completed" => {
                let receipt = &e["receipt"];
                let id = field(receipt, "task_id")?;
                if let Some(attempt) = self.state["tasks"][id]["lease"]["attempt_id"]
                    .as_str()
                    .map(str::to_owned)
                {
                    self.state["readers"]
                        .as_object_mut()
                        .unwrap()
                        .remove(&attempt);
                }
                self.state["tasks"][id]["state"] = json!("completed");
                self.deadlines.remove(id);
                self.state["submissions"][field(receipt, "submission_id")?] =
                    json!({"receipt":receipt,"lease":e["lease"]});
                self.state["commits"]
                    .as_array_mut()
                    .unwrap()
                    .push(receipt.clone());
            }
            "Sealed" => self.sealed = true,
            "ConsumerState" => {
                self.state["consumers"][field(e, "consumer_id")?] = e["state"].clone()
            }
            "BatchPlanned" => self.state["batches"][field(e, "batch_id")?] = e["batch"].clone(),
            "BatchReady" => {
                let b = &mut self.state["batches"][field(e, "batch_id")?];
                b["ready_ref"] = e["ready_ref"].clone();
                b["ready"] = json!(true);
                self.state["consumers"][field(e, "consumer_id")?] = e["state"].clone();
            }
            "ReaderDone" => {
                for lease in array(e, "leases")? {
                    self.state["readers"]
                        .as_object_mut()
                        .unwrap()
                        .remove(field(lease, "attempt_id")?);
                }
            }
            "ReaderRetired" => {
                self.state["readers"]
                    .as_object_mut()
                    .unwrap()
                    .retain(|_, r| r["lease"]["worker_id"] != e["worker_id"]);
            }
            "RetentionAdvanced" => {
                self.state["retired_prefix"] = e["prefix"].clone();
                self.state["retired_positions"] = e["positions"].clone();
            }
            "CheckpointReleased" => {
                self.state["checkpoints"]
                    .as_object_mut()
                    .unwrap()
                    .remove(field(e, "checkpoint_id")?);
            }
            "Checkpoint" => {
                self.state["checkpoints"][field(e, "checkpoint_id")?] = e["checkpoint"].clone()
            }
            kind => {
                return fail(
                    "UnsafeRecovery",
                    format!("Unknown journal event type: {kind}"),
                );
            }
        }
        for id in &affected {
            let task = &self.state["tasks"][id];
            if task["state"] == "pending" {
                self.pending.insert(self.positions[id], id.clone());
            }
            if task["state"] == "leased" {
                self.leased.insert(id.clone());
            }
            if let Some(usage) = self.usage_cache.as_mut() {
                Self::adjust_active(usage, task, true);
            }
        }
        if kind == "Completed" {
            if let Some(usage) = self.usage_cache.as_mut() {
                for (key, field) in [
                    ("records", "records"),
                    ("bytes", "payload_bytes"),
                    ("tokens", "tokens"),
                ] {
                    let n = number(&e["receipt"]["result_ref"], field);
                    usage["accepted_unprocessed"][key] =
                        json!(number(&usage["accepted_unprocessed"], key) + n);
                    usage[key] = json!(number(usage, key) + n);
                }
            }
        } else if matches!(kind, "ConsumerState" | "BatchReady" | "Checkpoint") {
            self.usage_cache = None;
        }
        if let Some(key) = e["request_key"].as_str() {
            self.state["requests"][key] = json!([e["request_digest"], e["response"]]);
        }
        Ok(())
    }
    fn adjust_active(usage: &mut Value, task: &Value, add: bool) {
        if task.is_null() || task["state"] != "leased" {
            return;
        }
        let control = boolean(&task["spec"], "control");
        let key = if control {
            "control_inflight"
        } else {
            "inflight"
        };
        let change = |n: u64, amount: u64| if add { n + amount } else { n - amount };
        usage[key] = json!(change(number(usage, key), 1));
        if !control {
            for key in ["records", "bytes", "tokens"] {
                usage[key] = json!(change(
                    number(usage, key),
                    number(&task["spec"], &format!("estimated_{key}"))
                ));
            }
        }
    }
    fn request(
        &self,
        operation: &str,
        id: &str,
        content: &Value,
    ) -> Result<(String, String, Value)> {
        if id.is_empty() || id.len() > 1024 {
            return fail("ValueError", "A bounded stable request ID is required");
        }
        let key = String::from_utf8(bytes(&json!([operation, id]))?).unwrap();
        let hash = digest(content)?;
        let prior = &self.state["requests"][&key];
        if !prior.is_null() && prior[0] != hash {
            return fail(
                "IdempotencyConflict",
                format!("{operation} request ID reused with different content"),
            );
        }
        Ok((key, hash, prior.get(1).cloned().unwrap_or(Value::Null)))
    }
    fn event(
        kind: &str,
        request: &(String, String, Value),
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
    fn validate(
        &mut self,
        reference: &Value,
        task: Option<&str>,
        attempt: Option<&str>,
    ) -> Result<Vec<Value>> {
        let result = self.store.config.validate(reference, task, attempt);
        if result
            .as_ref()
            .is_err_and(|e| e.kind == "StorageUnavailable")
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
        if reference.is_null() {
            return Ok(json!({"processed_positions":[],"finished_batches":[]}));
        }
        if let Some((r, p)) = &self.progress_cache
            && r == reference
        {
            return Ok(p.clone());
        }
        let members = self.validate(reference, None, None)?;
        if members.len() != 1 {
            return fail("InvalidReference", "Expected one JSON progress record");
        }
        let record = self.store.config.read(&members[0])?;
        if record.metadata["codec"] != "json.v1" {
            return fail("InvalidReference", "Expected JSON progress");
        }
        let p: Value = serde_json::from_slice(&record.payload)?;
        if p["version"] != 1
            || array(&p, "processed_positions")?
                .iter()
                .any(|v| v.as_u64().is_none())
            || array(&p, "finished_batches")?
                .iter()
                .any(|v| !v.is_string())
        {
            return fail("InvalidReference", "Invalid sparse consumer progress");
        }
        for key in ["processed_positions", "finished_batches"] {
            let a = array(&p, key)?;
            let unique: HashSet<_> = a.iter().map(|v| v.to_string()).collect();
            if unique.len() != a.len() {
                return fail("InvalidReference", "Duplicate progress entries");
            }
        }
        self.progress_cache = Some((reference.clone(), p.clone()));
        Ok(p)
    }
    fn covered(&self, batch: &Value) -> bool {
        vals(&self.state["checkpoints"]).any(|c| {
            c["batch_ids"]
                .as_array()
                .is_some_and(|a| a.contains(&batch["batch_id"]))
        })
    }
    pub fn usage(&mut self) -> Result<Value> {
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
            let n = accepted
                .iter()
                .map(|r| number(&r["result_ref"], field))
                .sum::<u64>();
            let r = ready
                .iter()
                .map(|b| number(&b["ready_ref"], field))
                .sum::<u64>();
            usage["accepted_unprocessed"][key] = json!(n);
            usage["train_ready"][key] = json!(r);
            usage[key] = json!(
                n + active
                    .iter()
                    .map(|t| number(&t["spec"], &format!("estimated_{key}")))
                    .sum::<u64>()
            );
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
    fn consumer_state(&mut self, args: &Value) -> Result<Value> {
        let fetched = number(args, "fetch_cursor");
        let processed = number(args, "processed_cursor");
        if processed > fetched || fetched > array(&self.state, "commits")?.len() as u64 {
            return fail("ValueError", "Consumer cursor order is invalid");
        }
        self.validate(&args["state_ref"], None, None)?;
        let mut state = json!({"version":1,"state_ref":args["state_ref"],"fetch_cursor":fetched,"processed_cursor":processed});
        if !args["progress_ref"].is_null() {
            self.validate(&args["progress_ref"], None, None)?;
            let progress = self.progress(&args["progress_ref"])?;
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
            state["progress_ref"] = args["progress_ref"].clone();
        }
        Ok(state)
    }
    pub fn call(
        &mut self,
        method: &str,
        args: &Value,
        now: f64,
        fault: &mut Fault<'_>,
    ) -> Result<Value> {
        if method != "state" && (self.journal.poisoned || self.journal.closed()) {
            return fail(
                "UnsafeRecovery",
                "Coordinator unavailable; stop owner and recover",
            );
        }
        let metadata = number(&self.limits, "metadata_bytes");
        match method {
            "release_task_reads" => {
                bounded(args, metadata)?;
                for lease in array(args, "leases")? {
                    let prior = &self.state["readers"][field(lease, "attempt_id")?];
                    if !prior.is_null() && prior["lease"] != *lease {
                        return fail("StaleAttempt", "Reader lease identity mismatch");
                    }
                }
                self.commit(
                    json!([{"type":"ReaderDone","leases":args["leases"]}]),
                    fault,
                )?;
                Ok(Value::Null)
            }
            "collect_garbage" => self.collect_garbage(fault),
            "release_checkpoint" => {
                self.commit(json!([{"type":"CheckpointReleased","checkpoint_id":field(args,"checkpoint_id")?}]), fault)?;
                Ok(Value::Null)
            }
            "submit_tasks" => {
                let tasks = array(args, "tasks")?;
                let producer = args["producer_id"].as_str();
                if producer.is_some() != !args["producer_state"].is_null() {
                    return fail("ValueError", "Producer ID/state must be provided together");
                }
                let content = if producer.is_some() {
                    json!({"tasks":tasks,"producer_id":producer,"producer_state":args["producer_state"]})
                } else {
                    json!(tasks)
                };
                bounded(&content, metadata)?;
                let request = self.request("submit", field(args, "request_id")?, &content)?;
                if !request.2.is_null() {
                    self.count("duplicate_submissions");
                    return Ok(request.2);
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
                for (control, limit) in [
                    (false, number(&self.limits, "pending_tasks")),
                    (true, number(&self.limits, "control_tasks")),
                ] {
                    let existing = vals(&self.state["tasks"])
                        .filter(|t| {
                            (t["state"] == "pending" || (control && t["state"] == "leased"))
                                && boolean(&t["spec"], "control") == control
                        })
                        .count();
                    if existing as u64
                        + tasks
                            .iter()
                            .filter(|t| boolean(t, "control") == control)
                            .count() as u64
                        > limit
                    {
                        return fail(
                            "ResourceLimitExceeded",
                            "Pending/control task budget exceeded",
                        );
                    }
                }
                let mut ids = vec![];
                let mut unique = HashSet::new();
                for t in tasks {
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
                        if estimate > number(&self.limits, &format!("accepted_{key}")) {
                            return fail(
                                "ResourceLimitExceeded",
                                "Task estimate exceeds output budget",
                            );
                        }
                    }
                    if !t["input_ref"].is_null() {
                        self.validate(&t["input_ref"], None, None)?;
                    }
                    ids.push(json!(id));
                }
                self.commit(json!([Self::event("Submitted",&request,json!(ids),json!({"tasks":tasks,"producer_id":producer,"producer_state":args["producer_state"]}))]),fault)?;
                Ok(json!(ids))
            }
            "acquire" => {
                let worker = field(args, "worker_id")?;
                let max = args["max_tasks"].as_u64().unwrap_or(1);
                if worker.is_empty()
                    || worker.len() > 1024
                    || max == 0
                    || max > number(&self.limits, "inflight_tasks")
                {
                    return fail("ValueError", "Invalid worker identity/max tasks");
                }
                let expired = self
                    .deadlines
                    .iter()
                    .filter(|(_, d)| **d <= now)
                    .map(|(id, _)| json!(id))
                    .collect::<Vec<_>>();
                if !expired.is_empty() {
                    self.commit(json!([{"type":"Expired","task_ids":expired}]), fault)?;
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
                if let Some(ids) = ids {
                    bounded(&json!(ids), metadata)?;
                }
                let prefix = args["task_prefix"].as_str();
                let mut usage = self.usage()?;
                let mut assignments = vec![];
                let mut assignment_bytes = 64u64;
                let mut pending = false;
                let candidates: Vec<&String> = if let Some(ids) = ids {
                    let mut selected = ids
                        .iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(|id| self.positions.get(id))
                        .filter_map(|p| self.pending.get_key_value(p))
                        .collect::<Vec<_>>();
                    selected.sort_by_key(|(position, _)| *position);
                    selected.into_iter().map(|(_, id)| id).collect()
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
                    pending = true;
                    if control {
                        if number(&usage, "control_inflight")
                            >= number(&self.limits, "control_tasks")
                        {
                            continue;
                        }
                    } else if number(&usage, "inflight") >= number(&self.limits, "inflight_tasks")
                        || number(&usage, "ready_bytes") >= number(&self.limits, "ready_bytes")
                        || ["records", "bytes", "tokens"].iter().any(|k| {
                            number(&usage, k) >= number(&self.limits, &format!("accepted_{k}"))
                                || number(&usage, k) + number(spec, &format!("estimated_{k}"))
                                    > number(&self.limits, &format!("accepted_{k}"))
                        })
                    {
                        continue;
                    }
                    let lease = json!({"run_id":self.store.config.run_id,"queue_id":self.queue_id,"task_id":id,"attempt_id":uid(),"generation":number(task,"generation")+1,"coordinator_epoch":self.epoch,"token":format!("{}{}",uid(),uid()),"worker_id":worker});
                    let assignment = json!({"task":spec,"lease":lease});
                    let size = bytes(&assignment)?.len() as u64 + 1;
                    if assignment_bytes + size > metadata {
                        if assignments.is_empty() {
                            return fail(
                                "ResourceLimitExceeded",
                                "Assignment exceeds control metadata limit",
                            );
                        };
                        break;
                    }
                    assignment_bytes += size;
                    assignments.push(assignment);
                    let key = if control {
                        "control_inflight"
                    } else {
                        "inflight"
                    };
                    usage[key] = json!(number(&usage, key) + 1);
                    for k in ["records", "bytes", "tokens"] {
                        usage[k] =
                            json!(number(&usage, k) + number(spec, &format!("estimated_{k}")));
                    }
                    if assignments.len() as u64 == max {
                        break;
                    }
                }
                if assignments.is_empty() {
                    return Ok(
                        json!({"status":if pending{"backpressured"}else{"empty"},"assignments":[]}),
                    );
                }
                let leases = assignments
                    .iter()
                    .map(|a| a["lease"].clone())
                    .collect::<Vec<_>>();
                bounded(&json!(leases), metadata)?;
                self.commit(json!([{"type":"Leased","leases":leases}]), fault)?;
                for lease in leases {
                    self.deadlines
                        .insert(field(&lease, "task_id")?.into(), now + self.lease_seconds);
                }
                Ok(json!({"status":"acquired","assignments":assignments}))
            }
            "save_task_progress_many" => {
                let updates = array(args, "updates")?;
                bounded(&json!(updates), metadata)?;
                let request =
                    self.request("progress_many", field(args, "request_id")?, &json!(updates))?;
                if !request.2.is_null() {
                    return Ok(request.2);
                }
                if updates.is_empty() {
                    return fail("ValueError", "Empty progress batch");
                }
                let mut events = vec![];
                let mut unique = HashSet::new();
                let mut reader = Reader::new(self.store.config.clone());
                for update in updates {
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
                    events.push(
                        json!({"type":"TaskProgress","task_id":id,"input_ref":update["input_ref"]}),
                    );
                }
                let last = events.last_mut().unwrap();
                last["request_key"] = json!(request.0);
                last["request_digest"] = json!(request.1);
                last["response"] = json!("saved");
                self.commit(json!(events), fault)?;
                Ok(json!("saved"))
            }
            "release_tasks" => {
                let leases = array(args, "leases")?;
                bounded(&json!(leases), metadata)?;
                let request =
                    self.request("release_tasks", field(args, "request_id")?, &json!(leases))?;
                if !request.2.is_null() {
                    return Ok(request.2);
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
                if let Some(last) = events.last_mut() {
                    last["request_key"] = json!(request.0);
                    last["request_digest"] = json!(request.1);
                    last["response"] = json!(response);
                    self.commit(json!(events), fault)?;
                }
                Ok(json!(response))
            }
            "heartbeat" => {
                let leases = array(args, "leases")?;
                bounded(&json!(leases), metadata)?;
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
                bounded(args, metadata)?;
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
                if number(reference, "payload_bytes") > number(&self.limits, "max_result_bytes")
                    || number(reference, "tokens") > number(&self.limits, "max_result_tokens")
                {
                    return fail(
                        "ResourceLimitExceeded",
                        "Per-task result upper bound exceeded",
                    );
                }
                self.validate(
                    reference,
                    Some(field(lease, "task_id")?),
                    Some(field(lease, "attempt_id")?),
                )?;
                let receipt = json!({"commit_id":uid(),"task_id":lease["task_id"],"submission_id":id,"digest":content,"position":array(&self.state,"commits")?.len(),"result_ref":reference});
                self.commit(
                    json!([{"type":"Completed","receipt":receipt,"lease":lease}]),
                    fault,
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
                bounded(&content, metadata)?;
                let request = self.request("fail", field(args, "request_id")?, &content)?;
                if !request.2.is_null() {
                    return Ok(request.2);
                }
                let task = self.valid_lease(lease, now)?;
                let response = json!(if retryable
                    && number(&task, "failures") + 1 < number(&task["spec"], "max_attempts")
                {
                    "pending"
                } else {
                    "failed"
                });
                self.commit(json!([Self::event("Failed",&request,response.clone(),json!({"task_ids":[lease["task_id"]],"failure":failure,"retryable":retryable}))]),fault)?;
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
                bounded(&content, metadata)?;
                let yielding = method == "yield_task";
                let request = self.request(
                    if yielding { "yield" } else { "task_progress" },
                    field(args, "request_id")?,
                    &content,
                )?;
                if !request.2.is_null() {
                    return Ok(request.2);
                }
                self.valid_lease(&args["lease"], now)?;
                self.validate(&args["input_ref"], None, None)?;
                let response = json!(if yielding { "pending" } else { "saved" });
                self.commit(
                    json!([Self::event(
                        if yielding { "Yielded" } else { "TaskProgress" },
                        &request,
                        response.clone(),
                        json!({"task_id":args["lease"]["task_id"],"input_ref":args["input_ref"]})
                    )]),
                    fault,
                )?;
                Ok(response)
            }
            "cancel_task" => {
                let id = field(args, "task_id")?;
                let request = self.request("cancel", field(args, "request_id")?, &json!(id))?;
                if !request.2.is_null() {
                    return Ok(request.2);
                }
                if terminal(&self.state["tasks"][id]["state"]) {
                    return fail("ValueError", "Cannot cancel terminal task");
                };
                self.commit(
                    json!([Self::event(
                        "Cancelled",
                        &request,
                        json!("cancelled"),
                        json!({"task_id":id})
                    )]),
                    fault,
                )?;
                Ok(json!("cancelled"))
            }
            "seal_input" => {
                let request = self.request("seal", field(args, "request_id")?, &Value::Null)?;
                if request.2.is_null() {
                    self.commit(
                        json!([Self::event("Sealed", &request, json!("sealed"), json!({}))]),
                        fault,
                    )?;
                }
                Ok(json!("sealed"))
            }
            "lookup_submission" => {
                Ok(self.state["submissions"][field(args, "submission_id")?]["receipt"].clone())
            }
            "task_status" => Ok(self.state["tasks"][field(args, "task_id")?].clone()),
            "producer_state" => Ok(self.state["producers"][field(args, "producer_id")?].clone()),
            "read_commits" => {
                let commits = array(&self.state, "commits")?;
                let cursor = number(args, "cursor") as usize;
                let limit = args["limit"].as_u64().unwrap_or(100) as usize;
                if cursor > commits.len() || limit == 0 || limit > 1000 {
                    return fail("ValueError", "Invalid cursor/page limit");
                }
                let mut end = (cursor + limit).min(commits.len());
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
                while bytes(&json!(&commits[cursor..end]))?.len() as u64 > metadata {
                    end = cursor + (end - cursor) / 2;
                }
                if end == cursor && cursor < commits.len() {
                    return fail(
                        "ResourceLimitExceeded",
                        "Commit exceeds control message limit",
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
                            self.commit(
                                json!([{"type":"Expired","task_ids":[lease["task_id"]]}]),
                                fault,
                            )?;
                        }
                        Err(e) => return Err(e),
                    }
                }
                self.commit(json!([{"type":"ReaderRetired","worker_id":worker}]), fault)?;
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
                if !request.2.is_null() {
                    return Ok(request.2);
                };
                self.commit(
                    json!([Self::event(
                        "ConsumerState",
                        &request,
                        state.clone(),
                        json!({"consumer_id":id,"state":state})
                    )]),
                    fault,
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
                self.validate(&args["plan_ref"], None, None)?;
                let batch = json!({"batch_id":batch_id,"consumer_id":id,"input_positions":positions,"plan_ref":args["plan_ref"],"ready":false});
                bounded(&batch, metadata)?;
                let request = self.request("plan_batch", batch_id, &batch)?;
                if !request.2.is_null() {
                    return Ok(self.state["batches"][batch_id].clone());
                };
                self.commit(
                    json!([Self::event(
                        "BatchPlanned",
                        &request,
                        batch.clone(),
                        json!({"batch_id":batch_id,"batch":batch})
                    )]),
                    fault,
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
                self.validate(&args["ready_ref"], None, None)?;
                let state = self.consumer_state(args)?;
                let content = json!({"batch_id":batch,"ready_ref":args["ready_ref"],"state":state});
                bounded(&content, metadata)?;
                let request = self.request("batch_ready", batch, &content)?;
                if !request.2.is_null() {
                    return Ok(request.2);
                };
                self.commit(json!([Self::event("BatchReady",&request,content.clone(),json!({"consumer_id":id,"batch_id":batch,"ready_ref":args["ready_ref"],"state":state}))]),fault)?;
                Ok(content)
            }
            "get_batch" => Ok(self.state["batches"][field(args, "batch_id")?].clone()),
            "register_checkpoint" => {
                let id = field(args, "checkpoint_id")?;
                let reference = &args["checkpoint_ref"];
                let members = self.validate(reference, None, None)?;
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
                self.validate(&checkpoint["consumer_state"], None, None)?;
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
                bounded(&content, metadata)?;
                let request = self.request("checkpoint", id, &content)?;
                if request.2.is_null() {
                    self.commit(
                        json!([Self::event(
                            "Checkpoint",
                            &request,
                            content.clone(),
                            json!({"checkpoint_id":id,"checkpoint":content})
                        )]),
                        fault,
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
        let prefix = number(state, "processed_cursor");
        let positions = array(progress, "processed_positions")?
            .iter()
            .filter_map(Value::as_u64)
            .collect::<HashSet<_>>();
        for receipt in array(&self.state, "commits")? {
            let position = number(receipt, "position");
            if position >= prefix && !positions.contains(&position) {
                references(&receipt["result_ref"], roots)?;
            }
        }
        let finished = array(progress, "finished_batches")?;
        for batch in vals(&self.state["batches"]) {
            if !finished.contains(&batch["batch_id"]) {
                references(batch, roots)?;
            }
        }
        Ok(())
    }
    fn collect_garbage(&mut self, fault: &mut Fault<'_>) -> Result<Value> {
        if !self.store.config.gc_enabled() {
            return fail("ValueError", "Online GC is not enabled");
        }
        let state = self.state["consumers"]["training"].clone();
        let progress = self.progress(&state["progress_ref"])?;
        let prefix = number(&state, "processed_cursor");
        let positions: HashSet<u64> = array(&progress, "processed_positions")?
            .iter()
            .filter_map(Value::as_u64)
            .collect();
        let mut roots = HashMap::new();
        for task in vals(&self.state["tasks"]).filter(|t| !terminal(&t["state"])) {
            references(&task["spec"]["input_ref"], &mut roots)?;
        }
        self.training_roots(&state, &progress, &mut roots)?;
        references(&self.state["readers"], &mut roots)?;
        references(&self.state["consumers"], &mut roots)?;
        references(&self.state["checkpoints"], &mut roots)?;
        // Persist expiration of replay before removing its storage ownership.
        self.commit(
            json!([{"type":"RetentionAdvanced","prefix":prefix,"positions":positions}]),
            fault,
        )?;
        let live = roots
            .keys()
            .map(|k| format!("{}{k}", self.storage_prefix))
            .collect::<HashSet<_>>();
        self.store
            .config
            .catalog(|c| {
                // commit() retained each immutable root's transitive pack set
                // before its WAL transaction. GC selects among those durable
                // owners; it must not reread every live payload graph while
                // holding the coordinator and cross-client catalog locks.
                // Missing ownership is a broken invariant, not a repairable
                // cache miss: fail before pruning or unlinking anything.
                c.require_retained(&live)?;
                c.prune(&self.store.config, &self.storage_prefix, &live, fault)?;
                c.collect(&self.store.config, fault)
            })?
            .ok_or_else(|| Error::new("ValueError", "Online GC is not enabled"))
    }
    pub fn close(&mut self) {
        self.draining = true;
        self.journal.close();
    }
}
fn field_value(v: &Value) -> Result<&str> {
    v.as_str()
        .ok_or_else(|| Error::new("CorruptData", "Expected identity string"))
}

fn references(value: &Value, result: &mut HashMap<String, Value>) -> Result<()> {
    if value.is_object()
        && value
            .get("manifest")
            .is_some_and(|v| v.get("segment").is_some())
        && value.get("digest").is_some()
    {
        result.insert(digest(value)?, value.clone());
    } else if let Some(object) = value.as_object() {
        for child in object.values() {
            references(child, result)?;
        }
    } else if let Some(array) = value.as_array() {
        for child in array {
            references(child, result)?;
        }
    }
    Ok(())
}
