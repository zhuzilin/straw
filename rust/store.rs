use crate::payload::InputRecord;
use crate::{Error, Fault, Result, array, bytes, digest, fail, field, hash, number, uid};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Instant,
};

// Kept as a batching hint for existing Python adapters, not an admission cap.
pub const MAX_RECORDS: usize = 10000;
const HEADER: &[u8] = b"SLMSEG01\x01\0\0\0";
const END: &[u8] = b"SLMEND01";
const TRAILER: u64 = 80;
#[derive(Clone)]
pub struct Record {
    pub metadata: Value,
    pub payload: std::sync::Arc<[u8]>,
}
#[derive(Clone)]
pub struct Config {
    pub root: PathBuf,
    pub run_id: String,
    pub codecs: HashSet<String>,
    pub max_record: u64,
    pub max_buffer: u64,
    pub target: u64,
    pub online_gc: bool,
    catalog: std::sync::Arc<std::sync::Mutex<Option<crate::gc::Catalog>>>,
}
pub struct Store {
    pub config: Config,
    writer: String,
    pack: Option<String>,
    last: Option<(String, String, Vec<Value>)>,
    pending: Option<(String, String, Value, PathBuf, usize)>,
    pub metrics: Value,
    closed: bool,
    pid: u32,
}
/// A bounded read operation over immutable extents. Only the last authenticated
/// index is retained; each record envelope and touched payload chunk is checked.
pub struct Reader {
    config: Config,
    last: Option<(Value, Value)>,
}
impl Reader {
    pub fn new(config: Config) -> Self {
        Self { config, last: None }
    }
    pub fn index(&mut self, segment: &Value) -> Result<&Value> {
        if self
            .last
            .as_ref()
            .is_none_or(|(reference, _)| reference != segment)
        {
            // Drop before validation: failed reads must never reuse an old index.
            self.last = None;
            let index = self.config.inspect(segment, false)?;
            self.last = Some((segment.clone(), index));
        }
        Ok(&self.last.as_ref().unwrap().1)
    }
    fn frame(&mut self, r: &Value) -> Result<(File, Value)> {
        self.index(&r["segment"])?;
        self.config
            .frame_with_index(r, &self.last.as_ref().unwrap().1)
    }
    pub fn envelope(&mut self, r: &Value) -> Result<Value> {
        let entries = array(self.index(&r["segment"])?, "records")?;
        let ordinal = r["ordinal"]
            .as_u64()
            .ok_or_else(|| Error::new("InvalidReference", "Invalid ordinal"))?
            as usize;
        let entry = entries
            .get(ordinal)
            .ok_or_else(|| Error::new("InvalidReference", "Ordinal outside segment"))?;
        Ok(entry["envelope"].clone())
    }
    pub fn read(&mut self, r: &Value) -> Result<Record> {
        let (file, env) = self.frame(r)?;
        Config::read_frame(file, env)
    }
    pub fn manifest(&mut self, r: &Value) -> Result<Value> {
        let (file, env) = self.frame(&r["manifest"])?;
        Config::decode_manifest(r, Config::read_frame(file, env)?)
    }
    pub fn read_tensor_range(
        &mut self,
        r: &Value,
        start: u64,
        stop: u64,
    ) -> Result<(Value, Vec<u8>, u64)> {
        let (f, env) = self.frame(r)?;
        Config::read_tensor_frame(f, env, start, stop)
    }
}
impl Config {
    pub fn new(
        root: &str,
        run: &str,
        codecs: Vec<String>,
        max_record: u64,
        max_buffer: u64,
        target: u64,
    ) -> Result<Self> {
        if run.is_empty() || max_record == 0 || max_buffer == 0 {
            return fail("ValueError", "Invalid run or memory limits");
        }
        let root = Path::new(root).to_path_buf();
        if !root.is_absolute() {
            return fail("ValueError", "Root must be absolute");
        }
        let mut codecs: HashSet<_> = codecs.into_iter().collect();
        codecs.extend(["record-set.v1".into(), "record-set.v2".into()]);
        Ok(Self {
            root,
            run_id: run.into(),
            codecs,
            max_record,
            max_buffer,
            target,
            online_gc: false,
            catalog: Default::default(),
        })
    }
    pub fn gc_enabled(&self) -> bool {
        self.online_gc || self.root.join("storage.log").exists()
    }
    pub fn catalog<T>(
        &self,
        call: impl FnOnce(&mut crate::gc::Catalog) -> Result<T>,
    ) -> Result<Option<T>> {
        if !self.gc_enabled() {
            return Ok(None);
        }
        let mut cache = self
            .catalog
            .lock()
            .map_err(|_| Error::new("UnsafeRecovery", "Storage catalog mutex poisoned"))?;
        if cache.is_none() {
            *cache = Some(crate::gc::Catalog::open(self)?);
        }
        cache.as_mut().unwrap().transaction(self, call).map(Some)
    }
    pub fn stage(&self, refs: &[Value], fault: &mut Fault<'_>) -> Result<()> {
        let roots = refs
            .iter()
            .map(|r| Ok((crate::gc::Catalog::staged(r)?, vec![r.clone()])))
            .collect::<Result<Vec<_>>>()?;
        self.catalog(|c| c.retain_many(self, &roots, fault))?;
        Ok(())
    }
    pub fn path(&self, relative: &str) -> Result<PathBuf> {
        storage_path(&self.root, relative)
    }
    fn open(&self, relative: &str) -> Result<File> {
        open_committed(&self.root, relative)
    }
    pub fn discover(&self, partition: &str) -> Result<Vec<Value>> {
        let base = self.path(&format!("raw/{partition}"))?;
        if !base.exists() {
            return Ok(vec![]);
        }
        let mut refs = vec![];
        for writer in fs::read_dir(base)? {
            let writer = writer?;
            if !writer.path().is_dir() {
                continue;
            }
            for entry in fs::read_dir(writer.path())? {
                let path = entry?.path();
                if path.extension().is_none_or(|ext| ext != "sealed") {
                    continue;
                }
                let relative = path
                    .strip_prefix(&self.root)
                    .map_err(|_| Error::new("InvalidReference", "Path escapes run root"))?
                    .to_string_lossy();
                let mut file = self.open(&relative)?;
                let size = file.metadata()?.len();
                if size < 12 + TRAILER {
                    return fail("CorruptData", "Segment size mismatch");
                }
                file.seek(SeekFrom::End(-(TRAILER as i64)))?;
                let footer = exact(&mut file, TRAILER as usize)?;
                let reference = json!({
                    "run_id": self.run_id,
                    "path": relative,
                    "segment_id": path.file_stem().unwrap().to_string_lossy(),
                    "size": size,
                    "checksum": hex(&footer[40..72]),
                    "version": 1,
                    "checksum_algorithm": "sha256",
                    "offset": 0,
                });
                self.inspect(&reference, false)?;
                refs.push(reference);
            }
        }
        Ok(refs)
    }
    pub fn inspect(&self, r: &Value, verify: bool) -> Result<Value> {
        let version = number(r, "version");
        let offset = match r.get("offset") {
            None => 0,
            Some(value) => value
                .as_u64()
                .ok_or_else(|| Error::new("InvalidReference", "Invalid extent offset"))?,
        };
        let size = number(r, "size");
        if ![1, 2].contains(&version) || r["checksum_algorithm"] != "sha256" {
            return fail(
                "UnsupportedSchema",
                "Unsupported segment reference version/checksum",
            );
        }
        let path = field(r, "path")?;
        if r["run_id"] != self.run_id
            || !path.ends_with(if version == 1 { ".sealed" } else { ".pack" })
        {
            return fail("InvalidReference", "Wrong run or uncommitted extent");
        }
        let mut f = self.open(path)?;
        let physical = f.metadata()?.len();
        if size < 12 + TRAILER
            || offset.checked_add(size).is_none_or(|n| n > physical)
            || (version == 1 && (offset != 0 || size != physical))
        {
            return fail(
                "CorruptData",
                format!(
                    "Segment size mismatch; path={path:?}, offset={offset}, extent_size={size}, physical_size={physical}"
                ),
            );
        }
        f.seek(SeekFrom::Start(offset))?;
        let header = exact(&mut f, 12)?;
        if header != HEADER {
            return fail(
                "CorruptData",
                format!(
                    "Invalid segment header; path={path:?}, offset={offset}, extent_size={size}, physical_size={physical}, observed_header={}, expected_header={}",
                    hex(&header),
                    hex(HEADER)
                ),
            );
        }
        f.seek(SeekFrom::Start(offset + size - TRAILER))?;
        let footer = exact(&mut f, TRAILER as usize)?;
        let length = u64::from_le_bytes(footer[..8].try_into().unwrap());
        if length > size - 12 - TRAILER
            || &footer[72..] != END
            || hex(&footer[40..72]) != r["checksum"]
        {
            return fail("CorruptData", "Invalid segment footer/checksum");
        }
        let index_offset = size - TRAILER - length;
        f.seek(SeekFrom::Start(offset + index_offset))?;
        let raw = exact(&mut f, length as usize)?;
        if Sha256::digest(&raw).as_slice() != &footer[8..40] {
            return fail("CorruptData", "Segment index checksum mismatch");
        }
        let index: Value = serde_json::from_slice(&raw)?;
        if index["version"] != 1 {
            return fail("UnsupportedSchema", "Unsupported index version");
        }
        if index["run_id"] != self.run_id || index["segment_id"] != r["segment_id"] {
            return fail("InvalidReference", "Segment identity mismatch");
        }
        let entries = array(&index, "records")?;
        if entries.is_empty() {
            return fail("CorruptData", "Invalid record count");
        }
        let mut expected = 12u64;
        for entry in entries {
            let env = &entry["envelope"];
            let n = number(env, "length");
            if env["version"] != 1 || !self.codecs.contains(field(env, "codec")?) {
                return fail("UnsupportedSchema", "Unsupported record codec/schema");
            }
            if number(entry, "offset") != expected {
                return fail("CorruptData", "Record index offset/length mismatch");
            }
            let meta = bytes(env)?;
            if u32::try_from(meta.len()).is_err() {
                return fail("CorruptData", "Record envelope exceeds u32 frame length");
            }
            expected = expected
                .checked_add(12 + meta.len() as u64 + n)
                .ok_or_else(|| Error::new("CorruptData", "Index overflow"))?;
        }
        if expected != index_offset {
            return fail("CorruptData", "Segment frame/index boundary mismatch");
        }
        if verify {
            f.seek(SeekFrom::Start(offset))?;
            let mut remaining = size - TRAILER;
            let mut h = Sha256::new();
            let mut buf = vec![0; 8 * 1024 * 1024];
            while remaining > 0 {
                let n = remaining.min(buf.len() as u64) as usize;
                f.read_exact(&mut buf[..n])?;
                h.update(&buf[..n]);
                remaining -= n as u64;
            }
            if format!("{:x}", h.finalize()) != r["checksum"] {
                return fail("CorruptData", "Segment payload checksum mismatch");
            }
        }
        Ok(index)
    }
    pub fn read(&self, r: &Value) -> Result<Record> {
        let (f, env) = self.frame(r)?;
        Self::read_frame(f, env)
    }
    fn read_frame(mut f: File, env: Value) -> Result<Record> {
        let payload = exact(&mut f, number(&env, "length") as usize)?;
        if hash(&payload) != env["checksum"] {
            return fail("CorruptData", "Record payload checksum mismatch");
        }
        Ok(Record {
            metadata: env,
            payload: payload.into(),
        })
    }
    fn frame(&self, r: &Value) -> Result<(File, Value)> {
        let index = self.inspect(&r["segment"], false)?;
        self.frame_with_index(r, &index)
    }
    fn frame_with_index(&self, r: &Value, index: &Value) -> Result<(File, Value)> {
        let segment = &r["segment"];
        let entries = array(index, "records")?;
        let ordinal = r["ordinal"]
            .as_u64()
            .ok_or_else(|| Error::new("InvalidReference", "Invalid ordinal"))?
            as usize;
        let entry = entries
            .get(ordinal)
            .ok_or_else(|| Error::new("InvalidReference", "Ordinal outside segment"))?;
        let env = entry["envelope"].clone();
        let encoded = bytes(&env)?;
        let mut f = self.open(field(segment, "path")?)?;
        f.seek(SeekFrom::Start(
            number(segment, "offset") + number(entry, "offset"),
        ))?;
        let frame = exact(&mut f, 12)?;
        let meta = u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize;
        let size = u64::from_le_bytes(frame[4..].try_into().unwrap());
        if meta != encoded.len()
            || size != number(&env, "length")
            || exact(&mut f, meta)? != encoded
        {
            return fail("CorruptData", "Record envelope differs from index");
        }
        Ok((f, env))
    }
    pub fn read_range(
        &self,
        r: &Value,
        start: u64,
        stop: u64,
        chunk: u64,
        checksums: &[String],
    ) -> Result<Vec<u8>> {
        Ok(self.read_range_counted(r, start, stop, chunk, checksums)?.0)
    }
    pub fn read_range_counted(
        &self,
        r: &Value,
        start: u64,
        stop: u64,
        chunk: u64,
        checksums: &[String],
    ) -> Result<(Vec<u8>, u64)> {
        let (f, env) = self.frame(r)?;
        Self::read_frame_range(f, &env, start, stop, chunk, checksums)
    }
    /// Check the tensor envelope and read its authenticated chunks in one pass.
    /// The caller still validates the returned dtype, shape and byte length.
    pub fn read_tensor_range(
        &self,
        r: &Value,
        start: u64,
        stop: u64,
    ) -> Result<(Value, Vec<u8>, u64)> {
        let (f, env) = self.frame(r)?;
        Self::read_tensor_frame(f, env, start, stop)
    }
    fn read_tensor_frame(
        f: File,
        env: Value,
        start: u64,
        stop: u64,
    ) -> Result<(Value, Vec<u8>, u64)> {
        if env["codec"] != "tensor.v1" {
            return fail("UnsupportedSchema", "Expected tensor.v1 record");
        }
        let metadata = &env["metadata"];
        let chunk = metadata["chunk_bytes"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or_else(|| Error::new("CorruptData", "Invalid tensor chunk size"))?;
        let checksums: Vec<String> = metadata["chunks"]
            .as_array()
            .ok_or_else(|| Error::new("CorruptData", "Invalid tensor chunk checksums"))?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| Error::new("CorruptData", "Invalid tensor chunk checksum"))
            })
            .collect::<Result<_>>()?;
        let (data, checked) = Self::read_frame_range(f, &env, start, stop, chunk, &checksums)?;
        Ok((env, data, checked))
    }
    fn read_frame_range(
        mut f: File,
        env: &Value,
        start: u64,
        stop: u64,
        chunk: u64,
        checksums: &[String],
    ) -> Result<(Vec<u8>, u64)> {
        let n = number(env, "length");
        if chunk == 0 || start > stop || stop > n || checksums.len() as u64 != n.div_ceil(chunk) {
            return fail("InvalidReference", "Invalid checksummed range");
        }
        let base = f.stream_position()?;
        let mut out = Vec::with_capacity((stop - start) as usize);
        let mut checked = 0;
        if start == stop {
            return Ok((out, checked));
        }
        for i in start / chunk..=(stop - 1) / chunk {
            let left = i * chunk;
            let len = (n - left).min(chunk);
            f.seek(SeekFrom::Start(base + left))?;
            let payload = exact(&mut f, len as usize)?;
            checked += len;
            if hash(&payload) != checksums[i as usize] {
                return fail("CorruptData", "Record chunk checksum mismatch");
            }
            out.extend_from_slice(
                &payload[start.saturating_sub(left) as usize..len.min(stop - left) as usize],
            );
        }
        Ok((out, checked))
    }
    pub fn manifest(&self, r: &Value) -> Result<Value> {
        Self::decode_manifest(r, self.read(&r["manifest"])?)
    }
    fn decode_manifest(r: &Value, record: Record) -> Result<Value> {
        let mut value: Value = serde_json::from_slice(&record.payload)?;
        match (record.metadata["codec"].as_str(), number(&value, "version")) {
            (Some("record-set.v1"), 1) => return Ok(value),
            (Some("record-set.v2"), 2) => {}
            _ => return fail("UnsupportedSchema", "Unsupported record-set schema/codec"),
        }
        let local = |n: &Value| -> Result<Value> {
            let i = n
                .as_u64()
                .ok_or_else(|| Error::new("InvalidReference", "Invalid local ordinal"))?;
            if i >= number(&r["manifest"], "ordinal") {
                return fail("InvalidReference", "Local reference must precede manifest");
            }
            Ok(json!({"segment":r["manifest"]["segment"],"ordinal":i}))
        };
        value["records"] = Value::Array(
            array(&value, "records")?
                .iter()
                .map(local)
                .collect::<Result<_>>()?,
        );
        for dep in value["dependencies"]
            .as_array_mut()
            .ok_or_else(|| Error::new("InvalidReference", "Invalid dependencies"))?
        {
            if let Some(n) = dep.get("local").cloned() {
                let reference = local(&n)?;
                dep.as_object_mut().unwrap().remove("local");
                dep["manifest"] = reference;
            }
        }
        Ok(value)
    }
    pub fn validate(
        &self,
        r: &Value,
        task: Option<&str>,
        attempt: Option<&str>,
    ) -> Result<Vec<Value>> {
        Reader::new(self.clone()).validate(r, task, attempt)
    }
}
impl Reader {
    pub fn validate(
        &mut self,
        r: &Value,
        task: Option<&str>,
        attempt: Option<&str>,
    ) -> Result<Vec<Value>> {
        self.validate_inner(r, task, attempt, &mut HashSet::new())
    }
    fn validate_inner(
        &mut self,
        r: &Value,
        task: Option<&str>,
        attempt: Option<&str>,
        seen: &mut HashSet<String>,
    ) -> Result<Vec<Value>> {
        for key in ["records", "payload_bytes", "tokens"] {
            if r[key].as_u64().is_none() {
                return fail("InvalidReference", "Invalid result usage");
            }
        }
        let key = digest(r)?;
        if seen.contains(&key) {
            return Ok(vec![]);
        }
        seen.insert(key);
        let manifest = self.manifest(r)?;
        let refs = array(&manifest, "records")?;
        if refs.len() as u64 != number(r, "records") {
            return fail("InvalidReference", "Record count mismatch");
        }
        let mut unique = HashSet::new();
        let mut envs = vec![];
        for member in refs {
            if !unique.insert(digest(member)?) {
                return fail("InvalidReference", "Repeated manifest member");
            }
            let env = self.envelope(member)?;
            if task.is_some_and(|t| env["metadata"]["task_id"] != t)
                || attempt.is_some_and(|a| env["metadata"]["attempt_id"] != a)
            {
                return fail("InvalidReference", "Result does not belong to task/attempt");
            }
            envs.push(env);
        }
        let deps = array(&manifest, "dependencies")?;
        let actual = digest(
            &json!({"records":envs,"dependencies":deps.iter().map(|d|d["digest"].clone()).collect::<Vec<_>>()}),
        )?;
        if r["digest"] != actual || manifest["digest"] != actual {
            return fail("InvalidReference", "Logical result digest mismatch");
        }
        if envs.iter().map(|e| number(e, "length")).sum::<u64>()
            + deps.iter().map(|d| number(d, "payload_bytes")).sum::<u64>()
            != number(r, "payload_bytes")
            || envs.iter().map(|e| number(e, "tokens")).sum::<u64>()
                + deps.iter().map(|d| number(d, "tokens")).sum::<u64>()
                != number(r, "tokens")
        {
            return fail("InvalidReference", "Result usage mismatch");
        }
        for dep in deps {
            self.validate_inner(dep, None, None, seen)?;
        }
        Ok(refs.clone())
    }
}
impl Store {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            writer: uid(),
            pack: None,
            last: None,
            pending: None,
            metrics: json!({"records":0,"payload_bytes":0,"segments":0,"buffer_bytes":0,"publish_seconds":[]}),
            closed: false,
            pid: std::process::id(),
        }
    }
    pub fn seal(&mut self, fault: &mut Fault<'_>) -> Result<()> {
        if self.pending.is_some() {
            return fail(
                "IndeterminateCommit",
                "Resolve pending publication before sealing",
            );
        }
        if let Some(path) = &self.pack {
            self.config
                .catalog(|c| c.pack(&self.config, path, true, fault))?;
        }
        self.pack = None;
        self.last = None;
        Ok(())
    }
    pub fn close(&mut self) -> Result<()> {
        // An uncertain append remains protected as an open pack for recovery.
        if self.pending.is_none() {
            self.seal(&mut |_| Ok(()))?;
        }
        self.closed = true;
        Ok(())
    }
    fn chunk_bytes(&self) -> usize {
        (self.config.max_buffer / 2).clamp(1, 4 * 1024 * 1024) as usize
    }
    fn envelope(&self, r: &InputRecord) -> Result<Value> {
        let mut e = r.metadata.clone();
        if field(&e, "record_id")?.is_empty() || !self.config.codecs.contains(field(&e, "codec")?) {
            return fail("UnsupportedSchema", "Unsupported record identity or codec");
        }
        if e["tokens"].as_u64().is_none() {
            return fail("ValueError", "Record tokens must be a nonnegative u64");
        }
        e["length"] = json!(r.payload.len());
        let mut hash = Sha256::new();
        r.payload.chunks(self.chunk_bytes(), &mut |chunk| {
            hash.update(chunk);
            Ok(())
        })?;
        e["checksum"] = json!(hex(&hash.finalize()));
        e["version"] = json!(1);
        if u32::try_from(bytes(&e)?.len()).is_err() {
            return fail("ValueError", "Record envelope exceeds u32 frame length");
        }
        Ok(e)
    }
    pub fn write(
        &mut self,
        records: &[Record],
        id: &str,
        fault: &mut Fault<'_>,
    ) -> Result<Vec<Value>> {
        let inputs: Vec<_> = records.iter().cloned().map(InputRecord::from).collect();
        self.write_inputs(&inputs, id, fault)
    }
    pub fn write_inputs(
        &mut self,
        records: &[InputRecord],
        id: &str,
        fault: &mut Fault<'_>,
    ) -> Result<Vec<Value>> {
        let envs = records
            .iter()
            .map(|r| self.envelope(r))
            .collect::<Result<Vec<_>>>()?;
        let refs = self.write_prepared(records, &envs, id, fault)?;
        self.config.stage(&refs, fault)?;
        Ok(refs)
    }
    fn finish(&self, r: &Value, partial: &Path, synced: bool, fault: &mut Fault<'_>) -> Result<()> {
        let path = self.config.path(field(r, "path")?)?;
        if number(r, "version") == 2 {
            if !synced {
                sync(&File::open(&path)?, fault)?;
            }
            sync_dir(path.parent().unwrap(), fault)?;
            return Ok(());
        }
        let publication = (|| {
            fault("before_publish")?;
            fs::rename(partial, &path)?;
            fault("after_publish")?;
            sync_dir(path.parent().unwrap(), fault)
        })();
        if let Err(e) = publication {
            if !["StorageUnavailable", "QuotaExceeded"].contains(&e.kind.as_str()) {
                return Err(e);
            }
            self.config.inspect(r, true).map_err(|e| {
                Error::new(
                    if e.kind == "CorruptData" {
                        "IdempotencyConflict"
                    } else {
                        "IndeterminateCommit"
                    },
                    e.message,
                )
            })?;
            sync_dir(path.parent().unwrap(), fault)?;
        }
        Ok(())
    }
    fn write_prepared(
        &mut self,
        records: &[InputRecord],
        envs: &[Value],
        id: &str,
        fault: &mut Fault<'_>,
    ) -> Result<Vec<Value>> {
        let started = Instant::now();
        if self.closed || self.pid != std::process::id() {
            return fail("RuntimeError", "Create a fresh writer after close/fork");
        }
        if id.is_empty() || id.len() > 1024 {
            return fail("ValueError", "Invalid submission ID");
        }
        if records.is_empty() {
            return fail("ValueError", "Cannot write an empty segment");
        }
        let total: u64 = records
            .iter()
            .zip(envs)
            .map(|(r, e)| r.payload.len() as u64 + bytes(e).unwrap().len() as u64 + 12)
            .sum();
        let mut ids = HashSet::new();
        for e in envs {
            if !ids.insert(field(e, "record_id")?) {
                return fail("ValueError", "Record IDs must be unique");
            }
        }
        let content = digest(&json!(envs))?;
        if let Some((old, hash, refs)) = &self.last
            && old == id
        {
            return if *hash == content {
                Ok(refs.clone())
            } else {
                fail(
                    "IdempotencyConflict",
                    "Submission ID reused with different content",
                )
            };
        }
        if let Some((old, hash, r, partial, count)) = &self.pending {
            if old != id {
                return fail(
                    "IndeterminateCommit",
                    "Resolve uncertain publication before writing another",
                );
            }
            if hash != &content {
                return fail(
                    "IdempotencyConflict",
                    "Uncertain publication retried with different content",
                );
            }
            if number(r, "version") == 2 {
                self.config.inspect(r, true)?;
            }
            self.finish(r, partial, false, fault)?;
            let refs = record_refs(r, *count);
            self.pending = None;
            self.last = Some((id.into(), content, refs.clone()));
            return Ok(refs);
        }
        let segment = uid();
        let directory = format!("raw/{}/{}", &self.writer[..2], self.writer);
        let relative = if self.config.target > 0 {
            let rotate = self.pack.as_ref().is_none_or(|p| {
                self.config
                    .path(p)
                    .and_then(|p| Ok(p.metadata()?.len()))
                    .unwrap_or(self.config.target)
                    + total
                    > self.config.target
            });
            if rotate {
                self.seal(fault)?;
                let next = format!("{directory}/{segment}.pack");
                self.config
                    .catalog(|c| c.pack(&self.config, &next, false, fault))?;
                self.pack = Some(next);
            }
            self.pack.clone().unwrap()
        } else {
            format!("{directory}/{segment}.sealed")
        };
        self.config
            .catalog(|c| c.pack(&self.config, &relative, false, fault))?;
        let path = self.config.path(&relative)?;
        mkdir(path.parent().unwrap(), fault)?;
        let partial = if self.config.target > 0 {
            path.clone()
        } else {
            path.with_extension("partial")
        };
        let mut opts = OpenOptions::new();
        opts.write(true);
        if self.config.target > 0 {
            opts.create(true).append(true);
        } else {
            opts.create_new(true);
        }
        let mut f = opts.open(&partial)?;
        let offset = f.seek(SeekFrom::End(0))?;
        let mut h = Sha256::new();
        let mut entries = vec![];
        write_hash(&mut f, &mut h, HEADER)?;
        fault("after_segment_header")?;
        for (record, env) in records.iter().zip(envs) {
            let pos = f.stream_position()? - offset;
            let meta = bytes(env)?;
            let mut frame = (meta.len() as u32).to_le_bytes().to_vec();
            frame.extend_from_slice(&(record.payload.len() as u64).to_le_bytes());
            write_hash(&mut f, &mut h, &frame)?;
            write_hash(&mut f, &mut h, &meta)?;
            let mut payload_hash = Sha256::new();
            record.payload.chunks(self.chunk_bytes(), &mut |chunk| {
                payload_hash.update(chunk);
                write_hash(&mut f, &mut h, chunk)?;
                fault("after_payload_chunk")
            })?;
            if hex(&payload_hash.finalize()) != field(env, "checksum")? {
                return fail("ValueError", "Input payload changed during publication");
            }
            entries.push(json!({"offset":pos,"envelope":env}));
            fault("after_record")?;
        }
        let index = bytes(
            &json!({"version":1,"run_id":self.config.run_id,"segment_id":segment,"records":entries}),
        )?;
        write_hash(&mut f, &mut h, &index)?;
        fault("before_footer")?;
        let checksum = h.finalize();
        f.write_all(&(index.len() as u64).to_le_bytes())?;
        f.write_all(&Sha256::digest(&index))?;
        f.write_all(&checksum)?;
        f.write_all(END)?;
        let size = f.stream_position()? - offset;
        let r = json!({"run_id":self.config.run_id,"path":relative,"segment_id":segment,"size":size,"checksum":hex(&checksum),"version":if self.config.target>0{2}else{1},"checksum_algorithm":"sha256","offset":offset});
        if self.config.target > 0 {
            self.pending = Some((
                id.into(),
                content.clone(),
                r.clone(),
                partial.clone(),
                records.len(),
            ))
        }
        sync(&f, fault)?;
        fault("after_data_sync")?;
        self.pending = Some((
            id.into(),
            content.clone(),
            r.clone(),
            partial.clone(),
            records.len(),
        ));
        self.finish(&r, &partial, true, fault)?;
        self.pending = None;
        let refs = record_refs(&r, records.len());
        self.last = Some((id.into(), content, refs.clone()));
        for (key, n) in [
            ("segments", 1),
            ("records", records.len() as u64),
            (
                "payload_bytes",
                records.iter().map(|r| r.payload.len() as u64).sum(),
            ),
        ] {
            self.metrics[key] = json!(number(&self.metrics, key) + n)
        }
        let times = self.metrics["publish_seconds"].as_array_mut().unwrap();
        times.push(json!(started.elapsed().as_secs_f64()));
        if times.len() > 4096 {
            times.remove(0);
        }
        Ok(refs)
    }
    /// Publish a manifest for existing immutable records, without copying payloads.
    pub fn record_set(
        &mut self,
        refs: &[Value],
        deps: &[Value],
        id: &str,
        fault: &mut Fault<'_>,
    ) -> Result<Value> {
        let operation = format!("publication:{}:{id}", self.writer);
        let held = refs.iter().chain(deps).cloned().collect::<Vec<_>>();
        self.config
            .catalog(|c| c.retain(&self.config, &operation, &held, fault))?;
        let mut reader = Reader::new(self.config.clone());
        let mut envelopes = Vec::new();
        let mut unique = HashSet::new();
        for r in refs {
            if !unique.insert(digest(r)?) {
                return fail("InvalidReference", "Repeated result member");
            }
            envelopes.push(reader.envelope(r)?);
        }
        for dep in deps {
            reader.validate(dep, None, None)?;
        }
        let logical = digest(
            &json!({"records":envelopes,"dependencies":deps.iter().map(|d|d["digest"].clone()).collect::<Vec<_>>()}),
        )?;
        let manifest = json!({"version":1,"records":refs,"dependencies":deps,"digest":logical});
        let record = Record {
            metadata: json!({"record_id":logical,"codec":"record-set.v1","metadata":{},"tokens":0}),
            payload: bytes(&manifest)?.into(),
        };
        let written = self.write(&[record], &format!("manifest:{id}"), fault)?;
        let result = json!({"manifest":written[0],"digest":logical,"records":refs.len(),
            "payload_bytes":envelopes.iter().map(|e|number(e,"length")).sum::<u64>()+deps.iter().map(|d|number(d,"payload_bytes")).sum::<u64>(),
            "tokens":envelopes.iter().map(|e|number(e,"tokens")).sum::<u64>()+deps.iter().map(|d|number(d,"tokens")).sum::<u64>()});
        self.config.stage(std::slice::from_ref(&result), fault)?;
        self.config.catalog(|c| {
            c.consume(&self.config, &written, fault)?;
            c.release(&self.config, &[operation], fault)
        })?;
        Ok(result)
    }

    pub fn publish(
        &mut self,
        groups: &Value,
        records: Vec<Record>,
        id: &str,
        fault: &mut Fault<'_>,
    ) -> Result<Vec<Value>> {
        self.publish_inputs(
            groups,
            records.into_iter().map(InputRecord::from).collect(),
            id,
            fault,
        )
    }

    pub fn publish_inputs(
        &mut self,
        groups: &Value,
        records: Vec<InputRecord>,
        id: &str,
        fault: &mut Fault<'_>,
    ) -> Result<Vec<Value>> {
        let groups = groups
            .as_array()
            .ok_or_else(|| Error::new("ValueError", "Expected publication array"))?;
        let operation = format!("publication:{}:{id}", self.writer);
        let held = groups
            .iter()
            .flat_map(|g| g["dependencies"].as_array().into_iter().flatten())
            .filter(|d| d.is_object())
            .cloned()
            .collect::<Vec<_>>();
        self.config
            .catalog(|c| c.retain(&self.config, &operation, &held, fault))?;
        let mut members = Vec::new();
        let mut envs = Vec::new();
        let mut descriptors: Vec<Value> = vec![];
        let mut reader = Reader::new(self.config.clone());
        for group in groups {
            let mut ordinals = vec![];
            let mut own_envs = vec![];
            for index in array(group, "records")? {
                let r = records
                    .get(
                        index
                            .as_u64()
                            .ok_or_else(|| Error::new("ValueError", "Invalid record index"))?
                            as usize,
                    )
                    .ok_or_else(|| Error::new("ValueError", "Invalid record index"))?;
                ordinals.push(members.len());
                let env = self.envelope(r)?;
                own_envs.push(env.clone());
                envs.push(env);
                members.push(r.clone());
            }
            let mut deps = vec![];
            for dep in array(group, "dependencies")? {
                if let Some(i) = dep.as_u64() {
                    if i as usize >= descriptors.len() {
                        return fail(
                            "InvalidReference",
                            "Local dependencies must precede publication",
                        );
                    }
                    deps.push(descriptors[i as usize].clone())
                } else {
                    reader.validate(dep, None, None)?;
                    deps.push(dep.clone());
                }
            }
            let logical = digest(
                &json!({"records":own_envs,"dependencies":deps.iter().map(|d|d["digest"].clone()).collect::<Vec<_>>()}),
            )?;
            descriptors.push(json!({"local":members.len(),"digest":logical,"records":ordinals.len(),"payload_bytes":own_envs.iter().map(|e|number(e,"length")).sum::<u64>()+deps.iter().map(|d|number(d,"payload_bytes")).sum::<u64>(),"tokens":own_envs.iter().map(|e|number(e,"tokens")).sum::<u64>()+deps.iter().map(|d|number(d,"tokens")).sum::<u64>()}));
            let payload = bytes(
                &json!({"version":2,"records":ordinals,"dependencies":deps,"digest":logical}),
            )?;
            let r = Record {
                metadata: json!({"record_id":format!("manifest:{}:{logical}",members.len()),"codec":"record-set.v2","metadata":{},"tokens":0}),
                payload: payload.into(),
            };
            let r = InputRecord::from(r);
            envs.push(self.envelope(&r)?);
            members.push(r);
        }
        let refs = self.write_prepared(&members, &envs, id, fault)?;
        let result = descriptors
            .into_iter()
            .map(|mut d| {
                let i = number(&d, "local") as usize;
                d.as_object_mut().unwrap().remove("local");
                d["manifest"] = refs[i].clone();
                d
            })
            .collect::<Vec<_>>();
        self.config.stage(&result, fault)?;
        self.config
            .catalog(|c| c.release(&self.config, &[operation], fault))?;
        if self.config.target == 0 {
            self.config.catalog(|c| {
                c.pack(
                    &self.config,
                    field(&refs[0]["segment"], "path")?,
                    true,
                    fault,
                )
            })?;
        }
        Ok(result)
    }
}
pub fn mkdir(path: &Path, fault: &mut Fault<'_>) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::new("ValueError", "Invalid directory"))?;
    mkdir(parent, fault)?;
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    };
    sync_dir(path, fault)?;
    sync_dir(parent, fault)
}
pub fn sync(file: &File, fault: &mut Fault<'_>) -> Result<()> {
    fault("before_file_sync")?;
    file.sync_all()?;
    Ok(())
}
pub fn sync_dir(path: &Path, fault: &mut Fault<'_>) -> Result<()> {
    fault("before_directory_sync")?;
    File::open(path)?.sync_all()?;
    Ok(())
}
fn write_hash(f: &mut File, h: &mut Sha256, v: &[u8]) -> Result<()> {
    f.write_all(v)?;
    h.update(v);
    Ok(())
}
fn record_refs(r: &Value, count: usize) -> Vec<Value> {
    (0..count)
        .map(|i| json!({"segment":r,"ordinal":i}))
        .collect()
}
fn exact(f: &mut File, n: usize) -> Result<Vec<u8>> {
    let mut v = vec![0; n];
    f.read_exact(&mut v).map_err(|e| {
        Error::new(
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                "CorruptData"
            } else {
                "StorageUnavailable"
            },
            e.to_string(),
        )
    })?;
    Ok(v)
}
fn hex(v: &[u8]) -> String {
    v.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn storage_path(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty()
        || relative.contains('\\')
        || Path::new(relative).is_absolute()
        || relative
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return fail("InvalidReference", "Path escapes run root");
    }
    let path = root.join(relative);
    let mut parent = path.as_path();
    while !parent.exists() {
        parent = parent
            .parent()
            .ok_or_else(|| Error::new("InvalidReference", "Invalid root"))?;
    }
    if root.exists() && !parent.canonicalize()?.starts_with(root.canonicalize()?) {
        return fail("InvalidReference", "Symlink escapes run root");
    }
    Ok(path)
}

pub fn open_committed(root: &Path, relative: &str) -> Result<File> {
    let p = storage_path(root, relative)?;
    for i in 0..5 {
        match File::open(&p) {
            Ok(f) => return Ok(f),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && i < 4 => {
                std::thread::sleep(std::time::Duration::from_millis(10 << i))
            }
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!()
}
