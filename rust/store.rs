use crate::payload::InputRecord;
use crate::{
    Error, Fault, Result, array, bytes, checked_sum, digest, fail, field, hash, number, uid,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

// Kept as a batching hint for existing Python adapters, not an admission cap.
pub const MAX_RECORDS: usize = 10000;
const HEADER: &[u8] = b"SLMSEG01\x01\0\0\0";
const END: &[u8] = b"SLMEND01";
const TRAILER: u64 = 80;
// Defensive tolerance for independently stale length/header/payload views.
// This is a fault-model assumption, not a demonstrated JuiceFS fsync behavior.
// Each retry reopens and authenticates all requested data.
// Internal retry signals are always converted to CorruptData at the boundary.
const EXTENT_VISIBILITY: &str = "ExtentVisibility";
const EXTENT_VISIBILITY_DELAYS_MS: [u64; 5] = [100, 250, 500, 1000, 2000];
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
    gc_required: std::sync::Arc<std::sync::atomic::AtomicBool>,
    catalog: std::sync::Arc<std::sync::Mutex<Option<crate::gc::Catalog>>>,
}
type PublicationDescription = (String, String, Value, PathBuf, usize);
type CompletedDescription = (String, String, Vec<Value>);
// Final catalog path, private write path and the pre-attempt file length.
type IncompleteDescription = ((String, PathBuf), u64);

pub struct Store {
    pub config: Config,
    writer: String,
    gate: Option<crate::writer_gate::Gate>,
    writer_registry: Option<crate::writer_gate::Registry>,
    pack: Option<String>,
    last: Option<crate::finalization::Handoff<CompletedDescription>>,
    pending: Option<crate::pending::Pending<PublicationDescription>>,
    incomplete: Option<crate::incomplete::State<IncompleteDescription>>,
    provisional_owner: Option<String>,
    preparations: BTreeSet<String>,
    pub metrics: Value,
    closed: bool,
    pid: u32,
}
/// A bounded read operation over immutable extents. Only the last authenticated
/// index is retained; each record envelope and touched payload chunk is checked.
pub struct Reader {
    config: Config,
    last: crate::record_index::Cache,
}
impl Reader {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            last: crate::record_index::Cache::new(),
        }
    }
    pub fn index(&mut self, segment: &Value) -> Result<&Value> {
        let mut source = &self.config;
        let mut observer = extent_observer(segment, extent_wait);
        crate::record_index::load(
            &mut self.last,
            &mut source,
            segment,
            &mut observer,
            &EXTENT_VISIBILITY_DELAYS_MS,
        )
        .map_err(extent_retry_error)?;
        Ok(self.last.index().unwrap())
    }
    fn index_once(&mut self, segment: &Value) -> Result<Option<File>> {
        self.last
            .load(&mut &self.config, segment)
            .map_err(|error| match error {
                crate::extent_retry::Failure::Visibility(error)
                | crate::extent_retry::Failure::Permanent(error) => error,
            })
    }
    fn frame(&mut self, r: &Value) -> Result<(File, Value)> {
        let opened = self.index_once(&r["segment"])?;
        self.config
            .frame_with_index(r, self.last.index().unwrap(), opened)
    }
    pub fn envelope(&mut self, r: &Value) -> Result<Value> {
        let mut source = &self.config;
        let mut observer = extent_observer(&r["segment"], extent_wait);
        crate::record_index::read_envelope(
            &mut self.last,
            &mut source,
            r,
            &mut observer,
            &EXTENT_VISIBILITY_DELAYS_MS,
        )
        .map_err(|error| match error {
            crate::record_index::ReadError::Load(error) => extent_retry_error(error),
            crate::record_index::ReadError::Select(error) => index_selection_error(error),
        })
    }
    pub fn read(&mut self, r: &Value) -> Result<Record> {
        retry_extent(&r["segment"], || {
            let (file, env) = self.frame(r)?;
            Config::read_frame(file, env)
        })
    }
    /// Authenticate the whole record without allocating a payload return value.
    pub(crate) fn verify(&mut self, r: &Value) -> Result<()> {
        retry_extent(&r["segment"], || {
            let (file, env) = self.frame(r)?;
            Config::verify_frame(file, env)
        })
    }
    pub fn manifest(&mut self, r: &Value) -> Result<Value> {
        crate::record_manifest::read(self, r).map_err(manifest_read_error)
    }
    pub fn read_tensor_range(
        &mut self,
        r: &Value,
        start: u64,
        stop: u64,
    ) -> Result<(Value, Vec<u8>, u64)> {
        self.read_tensor_range_checked(r, start, stop, None)
    }
    /// Validate an optional typed descriptor before payload allocation/I/O.
    pub fn read_tensor_range_checked(
        &mut self,
        r: &Value,
        start: u64,
        stop: u64,
        expected: Option<&Value>,
    ) -> Result<(Value, Vec<u8>, u64)> {
        retry_extent(&r["segment"], || {
            let (f, env) = self.frame(r)?;
            Config::read_tensor_frame(f, env, start, stop, expected)
        })
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
            gc_required: Default::default(),
            catalog: Default::default(),
        })
    }
    /// Discover existing GC pools by fresh open. Once required, GC stays
    /// required across Config clones; errors must never disable ownership.
    pub fn gc_enabled(&self) -> Result<bool> {
        self.gc_enabled_with_probe(|| File::open(self.root.join("storage.log")).map(|_| ()))
    }

    fn gc_enabled_with_probe(&self, probe: impl FnOnce() -> std::io::Result<()>) -> Result<bool> {
        use crate::discovery::{Decision, Observation, decide};
        use std::sync::atomic::Ordering;
        if decide(
            self.online_gc,
            self.gc_required.load(Ordering::Acquire),
            Observation::Unqueried,
        ) == Decision::Required
        {
            self.gc_required.store(true, Ordering::Release);
            return Ok(true);
        }
        let response = probe();
        let observation = match &response {
            Ok(()) => Observation::Present,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Observation::Absent,
            Err(_) => Observation::Failed,
        };
        // Another clone may have discovered/enabled GC while this probe ran.
        match decide(
            self.online_gc,
            self.gc_required.load(Ordering::Acquire),
            observation,
        ) {
            Decision::Required => {
                self.gc_required.store(true, Ordering::Release);
                Ok(true)
            }
            Decision::Legacy => Ok(false),
            Decision::Unavailable => match response {
                Err(e) => Err(e.into()),
                Ok(()) => fail("UnsafeRecovery", "Inconsistent GC discovery result"),
            },
            Decision::Probe => fail("UnsafeRecovery", "GC discovery did not finish"),
        }
    }
    fn with_catalog<T>(
        &self,
        call: impl FnOnce(&mut crate::gc::Catalog) -> Result<T>,
    ) -> Result<Option<T>> {
        if !self.gc_enabled()? {
            return Ok(None);
        }
        let mut cache = self
            .catalog
            .lock()
            .map_err(|_| Error::new("UnsafeRecovery", "Storage catalog mutex poisoned"))?;
        if cache.is_none() {
            // Auto-discovery opened an existing catalog. It must not silently
            // recreate that catalog if the name disappears before this open.
            *cache = Some(crate::gc::Catalog::open_with_create(self, self.online_gc)?);
        }
        call(cache.as_mut().unwrap()).map(Some)
    }
    pub fn catalog<T>(
        &self,
        call: impl FnOnce(&mut crate::gc::LockedCatalog<'_>) -> Result<T>,
    ) -> Result<Option<T>> {
        self.with_catalog(|catalog| catalog.transaction(self, call))
    }
    pub(crate) fn collect_queue(
        &self,
        prepared: crate::queue_gc::Prepared<'_>,
        fault: &mut Fault<'_>,
    ) -> Result<Option<Value>> {
        self.with_catalog(|catalog| catalog.collect_queue_transaction(self, prepared, fault))
    }

    pub fn stage(&self, refs: &[Value], fault: &mut Fault<'_>) -> Result<()> {
        let roots = refs
            .iter()
            .map(|r| Ok((crate::gc::Catalog::staged(r)?, vec![r.clone()])))
            .collect::<Result<Vec<_>>>()?;
        self.catalog(|c| c.retain_many(self, &roots, fault))?;
        Ok(())
    }
    fn finalize_publication(
        &self,
        roots: &[Value],
        segment: &Value,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        use crate::finalization::{self, Effect};
        let mut state = finalization::start(self.target == 0);
        while let Some(effect) = finalization::next(&state) {
            let outcome = match effect {
                Effect::Stage => self.stage(roots, fault),
                Effect::Seal => (|| {
                    fault("before_publication_seal")?;
                    self.catalog(|c| c.pack(self, field(segment, "path")?, true, fault))?;
                    fault("after_publication_seal")
                })(),
            };
            state = finalization::advance(state, outcome.is_ok());
            outcome?;
        }
        if !finalization::finished(&state) {
            return fail("UnsafeRecovery", "Publication finalization did not finish");
        }
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
        retry_extent(r, || self.inspect_once(r, verify))
    }
    fn inspect_once(&self, r: &Value, verify: bool) -> Result<Value> {
        Ok(self.inspect_opened(r, verify)?.1)
    }
    // Transfer this attempt's descriptor with its authenticated index. Readers
    // cache only the index: errors drop the file before the shared retry wait.
    fn inspect_opened(&self, r: &Value, verify: bool) -> Result<(File, Value)> {
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
        let mut f = self.open_visible_extent(path, offset, size, version)?;
        f.seek(SeekFrom::Start(offset + size - TRAILER))?;
        let footer = visible_exact(&mut f, TRAILER as usize)?;
        let length = u64::from_le_bytes(footer[..8].try_into().unwrap());
        if length > size - 12 - TRAILER
            || &footer[72..] != END
            || hex(&footer[40..72]) != r["checksum"]
        {
            return fail(EXTENT_VISIBILITY, "Invalid segment footer/checksum");
        }
        let index_offset = size - TRAILER - length;
        f.seek(SeekFrom::Start(offset + index_offset))?;
        let raw = visible_exact(&mut f, length as usize)?;
        if Sha256::digest(&raw).as_slice() != &footer[8..40] {
            return fail(EXTENT_VISIBILITY, "Segment index checksum mismatch");
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
            let n = env["length"]
                .as_u64()
                .ok_or_else(|| Error::new("CorruptData", "Invalid record length"))?;
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
            expected = crate::bounds::frame_end(expected, meta.len() as u64, n)
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
                f.read_exact(&mut buf[..n]).map_err(visibility_io)?;
                h.update(&buf[..n]);
                remaining -= n as u64;
            }
            if format!("{:x}", h.finalize()) != r["checksum"] {
                return fail(EXTENT_VISIBILITY, "Segment payload checksum mismatch");
            }
        }
        Ok((f, index))
    }
    fn open_visible_extent(
        &self,
        path: &str,
        offset: u64,
        size: u64,
        version: u64,
    ) -> Result<File> {
        let end = offset.checked_add(size).ok_or_else(|| {
            Error::new(
                "CorruptData",
                "Segment size mismatch: extent offset/size overflow",
            )
        })?;
        if size < 12 + TRAILER || (version == 1 && offset != 0) {
            return fail(
                "CorruptData",
                "Segment size mismatch: invalid extent size/offset",
            );
        }
        let mut f = self.open(path)?;
        let physical = f.metadata()?.len();
        if end > physical || (version == 1 && size != physical) {
            return fail(
                if end > physical {
                    EXTENT_VISIBILITY
                } else {
                    "CorruptData"
                },
                format!(
                    "Segment size mismatch; path={path:?}, offset={offset}, extent_size={size}, physical_size={physical}"
                ),
            );
        }
        f.seek(SeekFrom::Start(offset))?;
        let header = visible_exact(&mut f, 12)?;
        if header != HEADER {
            // Missing bytes may be scattered. A known nonzero wrong byte is
            // not a zero-fill visibility gap and still fails immediately.
            let zero_filled = header
                .iter()
                .zip(HEADER)
                .all(|(observed, expected)| *observed == 0 || observed == expected);
            return fail(
                if zero_filled {
                    EXTENT_VISIBILITY
                } else {
                    "CorruptData"
                },
                format!(
                    "Invalid segment header; path={path:?}, offset={offset}, extent_size={size}, physical_size={physical}, observed_header={}, expected_header={}",
                    hex(&header),
                    hex(HEADER)
                ),
            );
        }
        Ok(f)
    }
    pub fn read(&self, r: &Value) -> Result<Record> {
        retry_extent(&r["segment"], || {
            let (f, env) = self.frame(r)?;
            Self::read_frame(f, env)
        })
    }
    fn read_frame(mut f: File, env: Value) -> Result<Record> {
        let payload = visible_exact(&mut f, number(&env, "length") as usize)?;
        if hash(&payload) != env["checksum"] {
            return fail(EXTENT_VISIBILITY, "Record payload checksum mismatch");
        }
        Ok(Record {
            metadata: env,
            payload: payload.into(),
        })
    }
    fn verify_frame(mut f: File, env: Value) -> Result<()> {
        let length = number(&env, "length");
        let mut buffer = vec![0u8; crate::bounds::verification_capacity(length) as usize];
        let mut hasher = Sha256::new();
        let mut consumed = 0;
        while consumed < length {
            let (count, next) = crate::bounds::next_buffer(length, consumed, buffer.len() as u64)
                .ok_or_else(|| {
                Error::new("CorruptData", "Invalid verification buffer bounds")
            })?;
            let data = &mut buffer[..count as usize];
            f.read_exact(data).map_err(visibility_io)?;
            hasher.update(&*data);
            consumed = next;
        }
        if format!("{:x}", hasher.finalize()) != env["checksum"] {
            return fail(EXTENT_VISIBILITY, "Record payload checksum mismatch");
        }
        Ok(())
    }
    fn frame(&self, r: &Value) -> Result<(File, Value)> {
        let (file, index) = self.inspect_opened(&r["segment"], false)?;
        self.frame_with_index(r, &index, Some(file))
    }
    fn frame_with_index(
        &self,
        r: &Value,
        index: &Value,
        opened: Option<File>,
    ) -> Result<(File, Value)> {
        let segment = &r["segment"];
        let entry = crate::record_index::member(index, r).map_err(index_selection_error)?;
        let env = entry["envelope"].clone();
        let encoded = bytes(&env)?;
        let position = crate::bounds::frame_position(
            number(segment, "offset"),
            number(segment, "size"),
            number(entry, "offset"),
            encoded.len() as u64,
            number(&env, "length"),
        )
        .ok_or_else(|| Error::new("CorruptData", "Record frame outside extent"))?;
        let mut f = match opened {
            Some(file) => file,
            None => self.open(field(segment, "path")?)?,
        };
        f.seek(SeekFrom::Start(position))?;
        let frame = visible_exact(&mut f, 12)?;
        let meta = u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize;
        let size = u64::from_le_bytes(frame[4..].try_into().unwrap());
        if meta != encoded.len()
            || size != number(&env, "length")
            || visible_exact(&mut f, meta)? != encoded
        {
            return fail(EXTENT_VISIBILITY, "Record envelope differs from index");
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
        retry_extent(&r["segment"], || {
            let (f, env) = self.frame(r)?;
            Self::read_frame_range(f, &env, start, stop, chunk, checksums)
        })
    }
    /// Check the tensor envelope and read its authenticated chunks in one pass.
    /// The caller still validates the returned dtype, shape and byte length.
    pub fn read_tensor_range(
        &self,
        r: &Value,
        start: u64,
        stop: u64,
    ) -> Result<(Value, Vec<u8>, u64)> {
        self.read_tensor_range_checked(r, start, stop, None)
    }
    /// Validate an optional typed descriptor before payload allocation/I/O.
    pub fn read_tensor_range_checked(
        &self,
        r: &Value,
        start: u64,
        stop: u64,
        expected: Option<&Value>,
    ) -> Result<(Value, Vec<u8>, u64)> {
        retry_extent(&r["segment"], || {
            let (f, env) = self.frame(r)?;
            Self::read_tensor_frame(f, env, start, stop, expected)
        })
    }
    fn read_tensor_frame(
        f: File,
        env: Value,
        start: u64,
        stop: u64,
        expected: Option<&Value>,
    ) -> Result<(Value, Vec<u8>, u64)> {
        if env["codec"] != "tensor.v1" {
            return fail("UnsupportedSchema", "Expected tensor.v1 record");
        }
        let metadata = &env["metadata"];
        if let Some(expected) = expected {
            let layout = |metadata: &Value, length: &Value| -> Option<crate::typed::Layout> {
                Some(crate::typed::Layout {
                    shape: metadata["shape"]
                        .as_array()?
                        .iter()
                        .map(Value::as_u64)
                        .collect::<Option<_>>()?,
                    dtype: metadata["dtype"].as_str()?.to_owned(),
                    length: length.as_u64()?,
                })
            };
            let matches = layout(metadata, &env["length"])
                .zip(layout(expected, &expected["length"]))
                .is_some_and(|(actual, expected)| crate::typed::matches(&actual, &expected));
            if !matches {
                return fail(
                    "CorruptData",
                    "Typed tensor reference disagrees with its published envelope",
                );
            }
        }
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
        let (first, end) = crate::bounds::authenticated_span(n, start, stop, chunk)
            .ok_or_else(|| Error::new("InvalidReference", "Invalid checksummed range"))?;
        if checksums.len() as u64 != n.div_ceil(chunk) {
            return fail("InvalidReference", "Invalid checksummed range");
        }
        let base = f.stream_position()?;
        let mut out = Vec::new();
        let capacity = usize::try_from(stop - start)
            .map_err(|_| Error::new("ValueError", "Requested range exceeds address space"))?;
        out.try_reserve_exact(capacity).map_err(|e| {
            Error::new(
                "ValueError",
                format!("Cannot allocate requested range: {e}"),
            )
        })?;
        let mut checked = 0;
        if start == stop {
            return Ok((out, checked));
        }
        // Older or externally supplied metadata can use gigabyte chunks.
        // Keep verification scratch independent of authentication granularity.
        let mut buffer = vec![0u8; crate::bounds::verification_capacity(chunk) as usize];
        for i in first / chunk..=(end - 1) / chunk {
            let left = i * chunk;
            let len = (n - left).min(chunk);
            let offset = base
                .checked_add(left)
                .ok_or_else(|| Error::new("CorruptData", "Chunk offset overflow"))?;
            f.seek(SeekFrom::Start(offset))?;
            let mut hasher = Sha256::new();
            let mut consumed = 0;
            while consumed < len {
                let (count, next) = crate::bounds::next_buffer(len, consumed, buffer.len() as u64)
                    .ok_or_else(|| {
                        Error::new("CorruptData", "Invalid verification buffer bounds")
                    })?;
                let count = count as usize;
                let data = &mut buffer[..count];
                f.read_exact(data).map_err(visibility_io)?;
                hasher.update(&*data);
                let position = left + consumed;
                let from = crate::bounds::clip(start, position, count as u64) as usize;
                let to = crate::bounds::clip(stop, position, count as u64) as usize;
                if from < to {
                    out.extend_from_slice(&data[from..to]);
                }
                consumed = next;
            }
            checked += len;
            if format!("{:x}", hasher.finalize()) != checksums[i as usize] {
                return fail(EXTENT_VISIBILITY, "Record chunk checksum mismatch");
            }
        }
        Ok((out, checked))
    }
    pub fn manifest(&self, r: &Value) -> Result<Value> {
        Self::decode_manifest(r, self.read(&r["manifest"])?)
    }
    fn decode_manifest(r: &Value, record: Record) -> Result<Value> {
        crate::record_manifest::decode(r, &record.metadata, &record.payload).map_err(manifest_error)
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
struct ValidationDigest;
impl crate::closure_node::Digest for ValidationDigest {
    type Error = Error;
    fn digest(&mut self, value: &Value) -> Result<String> {
        digest(value)
    }
}
impl crate::closure::SourceModel for Reader {
    type Reference = Value;
}
impl crate::record_manifest::Reader for Reader {
    type Error = Error;
    fn record(&mut self, reference: &Value) -> Result<crate::queue_progress::Record> {
        let record = Reader::read(self, reference)?;
        Ok(crate::queue_progress::Record {
            metadata: record.metadata,
            payload: record.payload,
        })
    }
}
impl crate::record_manifest::Envelopes for Reader {
    type Error = Error;
    fn envelope(&mut self, reference: &Value) -> Result<Value> {
        Reader::envelope(self, reference)
    }
}
fn manifest_error(error: crate::record_manifest::Error) -> Error {
    use crate::record_manifest::Error as E;
    match error {
        E::Json(error) => error.into(),
        E::Schema => Error::new("UnsupportedSchema", "Unsupported record-set schema/codec"),
        E::Records => Error::new("InvalidReference", "Missing array records"),
        E::Dependencies => Error::new("InvalidReference", "Invalid dependencies"),
        E::Ordinal => Error::new("InvalidReference", "Invalid local ordinal"),
        E::Precedence => Error::new("InvalidReference", "Local reference must precede manifest"),
    }
}
fn manifest_read_error(error: crate::record_manifest::ReadError<Error>) -> Error {
    match error {
        crate::record_manifest::ReadError::Read(error) => error,
        crate::record_manifest::ReadError::Decode(error) => manifest_error(error),
    }
}
fn validation_error(
    error: crate::reference_node::Error<crate::record_manifest::ReadError<Error>, Error>,
) -> Error {
    use crate::reference_node::Error as E;
    match error {
        E::Read(error) => manifest_read_error(error),
        E::Digest(error) => error,
        E::Usage => Error::new("InvalidReference", "Invalid result usage"),
        E::Array(name) => Error::new("InvalidReference", format!("Missing array {name}")),
        E::Count => Error::new("InvalidReference", "Record count mismatch"),
        E::Repeated => Error::new("InvalidReference", "Repeated manifest member"),
        E::Scope => Error::new("InvalidReference", "Result does not belong to task/attempt"),
        E::Logical => Error::new("InvalidReference", "Logical result digest mismatch"),
        E::Overflow => Error::new("InvalidReference", "Unsigned 64-bit accounting overflow"),
        E::UsageMismatch => Error::new("InvalidReference", "Result usage mismatch"),
    }
}
impl Reader {
    pub fn validate(
        &mut self,
        r: &Value,
        task: Option<&str>,
        attempt: Option<&str>,
    ) -> Result<Vec<Value>> {
        let mut records = crate::record_manifest::Adapter::new(self);
        let mut source = crate::reference_node::Adapter::new(&mut records, ValidationDigest);
        crate::reference_validation::validate(&mut source, &mut ValidationDigest, r, task, attempt)
            .map_err(|error| match error {
                crate::reference_validation::Error::Read(error) => validation_error(error),
                crate::reference_validation::Error::Digest(error) => error,
            })
    }
    pub(crate) fn validate_node(
        &mut self,
        r: &Value,
        task: Option<&str>,
        attempt: Option<&str>,
    ) -> Result<(Vec<Value>, Vec<Value>)> {
        let mut records = crate::record_manifest::Adapter::new(self);
        crate::reference_node::validate(&mut records, &mut ValidationDigest, r, task, attempt)
            .map_err(validation_error)
    }
}

impl Store {
    fn check_process(&self) -> Result<()> {
        if !crate::incomplete::writer_process(self.pid, std::process::id()) {
            return fail(
                "RuntimeError",
                "Writer cannot be reused after fork; create a fresh writer",
            );
        }
        Ok(())
    }
    fn unresolved(&self) -> bool {
        self.pending.is_some() || self.last.as_ref().is_some_and(|state| state.active)
    }
    fn check_request(&self, api: crate::finalization::Api, id: &str) -> Result<()> {
        self.check_process()?;
        if self.closed {
            return fail("RuntimeError", "Create a fresh writer after close/fork");
        }
        if let Some(state) = &self.last
            && !crate::finalization::allows(state, api, state.value.0 == id)
        {
            return fail(
                "IndeterminateCommit",
                "Resolve the pending publication through the same API and submission ID",
            );
        }
        Ok(())
    }
    fn acknowledge_publication(&mut self) {
        if let Some(state) = self.last.as_mut() {
            crate::finalization::acknowledge(state);
        }
    }
    pub fn new(config: Config) -> Self {
        Self {
            config,
            writer: uid(),
            gate: None,
            writer_registry: None,
            pack: None,
            last: None,
            pending: None,
            incomplete: None,
            provisional_owner: None,
            preparations: BTreeSet::new(),
            metrics: json!({"records":0,"payload_bytes":0,"segments":0,"buffer_bytes":0,"publish_seconds":[]}),
            closed: false,
            pid: std::process::id(),
        }
    }
    pub fn seal(&mut self, fault: &mut Fault<'_>) -> Result<()> {
        self.check_process()?;
        let permit = self.enter_writer(fault)?;
        self.clean_provisional(&permit, fault)?;
        self.seal_pack(&permit, fault)
    }
    // Rotation is inside a preparation attempt: its dependency hold must live
    // across sealing the preceding pack and writing the replacement.
    fn seal_pack(
        &mut self,
        permit: &crate::writer_gate::Permit,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.check_process()?;
        self.clean_incomplete(permit, fault)?;
        if self.unresolved() {
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
        self.check_process()?;
        if self.closed {
            return Ok(());
        }
        let permit = self.enter_writer(&mut |_| Ok(()))?;
        self.clean_provisional(&permit, &mut |_| Ok(()))?;
        // An uncertain append remains protected as an open pack for recovery.
        if !self.unresolved() {
            self.seal_pack(&permit, &mut |_| Ok(()))?;
        }
        self.closed = true;
        Ok(())
    }
    fn chunk_bytes(&self) -> usize {
        (self.config.max_buffer / 2).clamp(1, 4 * 1024 * 1024) as usize
    }
    fn enter_writer(&mut self, fault: &mut Fault<'_>) -> Result<crate::writer_gate::Permit> {
        if self.gate.is_none() {
            self.gate = Some(crate::writer_gate::Gate::create(
                &self.config.root,
                &self.config.run_id,
                &self.writer,
                fault,
            )?);
        }
        let mut permit = self
            .gate
            .as_ref()
            .unwrap()
            .enter(&self.config.root, &self.config.run_id)?;
        // Recover while holding the same incarnation gate; a caught unwind or
        // uncertain registry append never reuses a cached partial replay.
        self.writer_registry = Some(permit.registry(fault)?);
        Ok(permit)
    }
    pub fn writer_id(&self) -> &str {
        &self.writer
    }
    fn hold_preparation(
        &mut self,
        owner: &str,
        refs: &[Value],
        fault: &mut Fault<'_>,
    ) -> Result<u64> {
        let operation =
            self.writer_registry
                .as_mut()
                .unwrap()
                .begin(owner, Some(owner), refs, fault)?;
        if !self.unresolved() {
            // Save before the append: a failed hold may already be durable.
            self.preparations.insert(owner.to_string());
        }
        self.config
            .catalog(|c| c.hold(&self.config, owner, refs, fault))?;
        Ok(operation)
    }
    fn clean_provisional(
        &mut self,
        _permit: &crate::writer_gate::Permit,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        loop {
            if self.provisional_owner.is_none() {
                self.provisional_owner = self.preparations.pop_first();
            }
            let Some(owner) = self.provisional_owner.as_ref() else {
                return Ok(());
            };
            let outcome = (|| {
                fault("before_provisional_release")?;
                self.config.catalog(|c| {
                    c.release_preparation(&self.config, std::slice::from_ref(owner), fault)
                })?;
                fault("after_provisional_release")?;
                self.writer_registry
                    .as_mut()
                    .unwrap()
                    .done_owner(owner, fault)
            })();
            crate::holding::finish(&mut self.provisional_owner, outcome.is_ok());
            outcome.map_err(|error| {
                Error::new(
                    "IndeterminateCommit",
                    format!(
                        "Preparation owner cleanup must finish before reuse: {}",
                        error.message
                    ),
                )
            })?;
        }
    }
    fn resume_provisional_cleanup(
        &mut self,
        permit: &crate::writer_gate::Permit,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        // An explicit seal/close starts abandonment. Before that, failed
        // preparations remain retryable, including across other requests.
        if self.provisional_owner.is_some() {
            self.clean_provisional(permit, fault)?;
        }
        Ok(())
    }
    fn clean_incomplete(
        &mut self,
        _permit: &crate::writer_gate::Permit,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        use crate::incomplete::{self, Effect};
        let Some(state) = self.incomplete.as_mut() else {
            return Ok(());
        };
        incomplete::restart(state);
        let ((relative, path), prefix) = state.value.clone();
        while let Some(effect) = incomplete::next(state) {
            let outcome = (|| match effect {
                Effect::Truncate => {
                    fault("before_incomplete_truncate")?;
                    OpenOptions::new()
                        .write(true)
                        .open(&path)?
                        .set_len(prefix)?;
                    fault("after_incomplete_truncate")
                }
                Effect::SyncFile => sync(&File::open(&path)?, fault),
                Effect::UnlinkPartial => {
                    fault("before_incomplete_unlink")?;
                    match fs::remove_file(&path) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e.into()),
                    }
                    fault("after_incomplete_unlink")
                }
                Effect::SyncParent => {
                    // Setup can fail before this private parent ever existed.
                    // Absence is not a deletion barrier; repair before sealing.
                    let parent = path.parent().unwrap();
                    fault("before_directory_sync")?;
                    let opened = File::open(parent);
                    let missing = opened
                        .as_ref()
                        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
                    match crate::directory::parent_action(opened.is_ok(), missing) {
                        crate::directory::ParentAction::Sync => {
                            opened?.sync_all()?;
                            Ok(())
                        }
                        crate::directory::ParentAction::RecreateAndSync => {
                            fault("before_incomplete_parent_repair")?;
                            mkdir(parent, fault)
                        }
                        crate::directory::ParentAction::Fail => Err(opened.err().unwrap().into()),
                    }
                }
                Effect::SealFinal => {
                    fault("before_incomplete_seal")?;
                    self.config
                        .catalog(|c| c.seal_unpublished(&self.config, &relative, fault))?;
                    fault("after_incomplete_seal")
                }
            })();
            incomplete::advance(state, outcome.is_ok());
            if let Err(error) = outcome {
                return fail(
                    "IndeterminateCommit",
                    format!(
                        "Incomplete publication cleanup must finish before reuse: {}",
                        error.message
                    ),
                );
            }
        }
        if !incomplete::finished(state) {
            return fail("UnsafeRecovery", "Incomplete cleanup did not finish");
        }
        self.incomplete = None;
        Ok(())
    }
    fn envelope(&self, r: &InputRecord) -> Result<Value> {
        let mut e = r.metadata.clone();
        if field(&e, "record_id")?.is_empty() || !self.config.codecs.contains(field(&e, "codec")?) {
            return fail("UnsupportedSchema", "Unsupported record identity or codec");
        }
        if e["tokens"].as_u64().is_none() {
            return fail("ValueError", "Record tokens must be a nonnegative u64");
        }
        let length = r.payload.len() as u64;
        e["length"] = json!(length);
        let mut hash = Sha256::new();
        crate::payload::visit_exact(
            r.payload.as_ref(),
            self.chunk_bytes(),
            length,
            &mut |chunk| {
                hash.update(chunk);
                Ok(())
            },
        )?;
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
        use crate::finalization::Api;
        self.check_request(Api::Records, id)?;
        let permit = self.enter_writer(fault)?;
        self.resume_provisional_cleanup(&permit, fault)?;
        let operation_key = crate::writer_gate::operation_key(&self.writer, Api::Records, id);
        let operation =
            self.writer_registry
                .as_mut()
                .unwrap()
                .begin(&operation_key, None, &[], fault)?;
        let envs = records
            .iter()
            .map(|r| self.envelope(r))
            .collect::<Result<Vec<_>>>()?;
        let refs = self.write_prepared(records, &envs, id, Api::Records, &permit, fault)?;
        self.writer_registry
            .as_mut()
            .unwrap()
            .results(operation, &refs, &[], fault)?;
        self.config
            .finalize_publication(&refs, &refs[0]["segment"], fault)?;
        self.writer_registry
            .as_mut()
            .unwrap()
            .done_key(&operation_key, fault)?;
        fault("after_publication_receipt")?;
        self.acknowledge_publication();
        Ok(refs)
    }
    fn finish(
        &self,
        r: &Value,
        partial: &Path,
        synced: bool,
        retry: bool,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        let path = self.config.path(field(r, "path")?)?;
        if number(r, "version") == 2 {
            if retry {
                self.config.inspect(r, true)?;
            }
            if retry || !synced {
                sync(&File::open(&path)?, fault)?;
            }
            sync_dir(path.parent().unwrap(), fault)?;
            return Ok(());
        }
        if !synced {
            // A prior sync failure may have left only volatile bytes. Rename
            // and directory sync cannot substitute for this file barrier.
            // If an earlier rename took effect, reopen the same final path.
            let file = match File::open(partial) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => File::open(&path)?,
                Err(e) => return Err(e.into()),
            };
            sync(&file, fault)?;
        }
        let publication = (|| {
            fault("before_publish")?;
            fs::rename(partial, &path)?;
            fault("after_publish")?;
            if retry {
                self.config.inspect(r, true)?;
            }
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
        api: crate::finalization::Api,
        permit: &crate::writer_gate::Permit,
        fault: &mut Fault<'_>,
    ) -> Result<Vec<Value>> {
        let started = Instant::now();
        if self.closed || self.pid != std::process::id() {
            return fail("RuntimeError", "Create a fresh writer after close/fork");
        }
        self.clean_incomplete(permit, fault)?;
        if id.is_empty() || id.len() > 1024 {
            return fail("ValueError", "Invalid submission ID");
        }
        if records.is_empty() {
            return fail("ValueError", "Cannot write an empty segment");
        }
        let mut total = 0u64;
        for e in envs {
            total = checked_sum(
                [total, number(e, "length"), bytes(e)?.len() as u64, 12],
                "ValueError",
            )?;
        }
        let mut ids = HashSet::new();
        for e in envs {
            if !ids.insert(field(e, "record_id")?) {
                return fail("ValueError", "Record IDs must be unique");
            }
        }
        let content = digest(&json!(envs))?;
        if let Some(pending) = &self.pending {
            let (old, hash, r, partial, count) = &pending.value;
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
            self.finish(
                r,
                partial,
                !crate::pending::needs_sync(pending),
                true,
                fault,
            )?;
            let refs = record_refs(r, *count);
            self.pending = None;
            return Ok(refs);
        }
        if let Some(state) = self.last.as_mut()
            && crate::finalization::same_api(state.api, api)
            && state.value.0 == id
        {
            if state.value.1 != content {
                return fail(
                    "IdempotencyConflict",
                    "Submission ID reused with different content",
                );
            }
            // A previously acknowledged reference can be replayed or rejected
            // after release/GC without turning it into an unresolvable new
            // operation. A failed first handoff is already active here.
            return Ok(state.value.2.clone());
        }
        let segment = uid();
        let directory = format!("raw/{}/{}", &self.writer[..2], self.writer);
        let (relative, fresh) = if self.config.target > 0 {
            let rotate = self.pack.as_ref().is_none_or(|p| {
                self.config
                    .path(p)
                    .and_then(|p| Ok(p.metadata()?.len()))
                    .unwrap_or(self.config.target)
                    .checked_add(total)
                    .is_none_or(|size| size > self.config.target)
            });
            if rotate {
                self.seal_pack(permit, fault)?;
                (format!("{directory}/{segment}.pack"), true)
            } else {
                (self.pack.clone().unwrap(), false)
            }
        } else {
            (format!("{directory}/{segment}.sealed"), true)
        };
        let path = self.config.path(&relative)?;
        let partial = if self.config.target > 0 {
            path.clone()
        } else {
            path.with_extension("partial")
        };
        if fresh {
            self.incomplete = Some(crate::incomplete::preparing((
                relative.clone(),
                partial.clone(),
            )));
        }
        let prepared = (|| -> Result<(File, u64)> {
            self.config
                .catalog(|c| c.pack(&self.config, &relative, false, fault))?;
            mkdir(path.parent().unwrap(), fault)?;
            let mut opts = OpenOptions::new();
            opts.write(true);
            if self.config.target > 0 {
                opts.create(true).append(true);
            } else {
                opts.create_new(true);
            }
            fault("before_publication_open")?;
            let mut f = opts.open(&partial)?;
            fault("after_publication_open")?;
            let offset = if self.config.target > 0 {
                fault("before_publication_offset")?;
                f.seek(SeekFrom::End(0))?
            } else {
                0
            };
            Ok((f, offset))
        })();
        let (mut f, offset) = match prepared {
            Ok(value) => value,
            Err(error) => {
                self.clean_incomplete(permit, fault)?;
                return Err(error);
            }
        };
        if self.config.target > 0 {
            self.pack = Some(relative.clone());
        }
        // From the first byte through the last position query, no reference to
        // this extent has escaped. Keep the exact rollback point until a full
        // description is transferred to pending sync/publication handling.
        if let Some(state) = self.incomplete.as_mut() {
            crate::incomplete::capture_prefix(state, offset, self.config.target == 0);
        } else {
            self.incomplete = Some(crate::incomplete::start(
                ((relative.clone(), partial.clone()), offset),
                self.config.target == 0,
            ));
        }
        let written = (|| -> Result<Value> {
            let mut h = Sha256::new();
            let mut entries = vec![];
            write_hash(&mut f, &mut h, HEADER)?;
            fault("after_segment_header")?;
            for (record, env) in records.iter().zip(envs) {
                let pos = f.stream_position()? - offset;
                let meta = bytes(env)?;
                let mut frame = (meta.len() as u32).to_le_bytes().to_vec();
                let length = number(env, "length");
                frame.extend_from_slice(&length.to_le_bytes());
                write_hash(&mut f, &mut h, &frame)?;
                write_hash(&mut f, &mut h, &meta)?;
                let mut payload_hash = Sha256::new();
                crate::payload::visit_exact(
                    record.payload.as_ref(),
                    self.chunk_bytes(),
                    length,
                    &mut |chunk| {
                        payload_hash.update(chunk);
                        write_hash(&mut f, &mut h, chunk)?;
                        fault("after_payload_chunk")
                    },
                )?;
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
            Ok(
                json!({"run_id":self.config.run_id,"path":relative,"segment_id":segment,"size":size,"checksum":hex(&checksum),"version":if self.config.target>0{2}else{1},"checksum_algorithm":"sha256","offset":offset}),
            )
        })();
        let r = match written {
            Ok(r) => r,
            Err(error) => {
                drop(f);
                self.clean_incomplete(permit, fault)?;
                return Err(error);
            }
        };
        let refs = record_refs(&r, records.len());
        self.last = Some(crate::finalization::handoff(
            (id.into(), content.clone(), refs.clone()),
            api,
        ));
        // The complete handoff now owns these dependencies. Preparation error
        // cleanup must never remove protection needed by its uncertain retry.
        if !crate::finalization::same_api(api, crate::finalization::Api::Records) {
            self.preparations
                .remove(&crate::writer_gate::operation_key(&self.writer, api, id));
        }
        self.pending = Some(crate::pending::start((
            id.into(),
            content.clone(),
            r.clone(),
            partial.clone(),
            records.len(),
        )));
        self.incomplete = None;
        let outcome = sync(&f, fault);
        crate::pending::record_sync(self.pending.as_mut().unwrap(), outcome.is_ok());
        outcome?;
        fault("after_data_sync")?;
        self.finish(&r, &partial, true, false, fault)?;
        self.pending = None;
        for (key, n) in [
            ("segments", 1),
            ("records", records.len() as u64),
            (
                "payload_bytes",
                envs.iter().map(|e| number(e, "length")).sum(),
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
        use crate::finalization::Api;
        self.check_request(Api::RecordSet, id)?;
        let permit = self.enter_writer(fault)?;
        self.resume_provisional_cleanup(&permit, fault)?;
        self.record_set_attempt(refs, deps, id, &permit, fault)
    }
    fn record_set_attempt(
        &mut self,
        refs: &[Value],
        deps: &[Value],
        id: &str,
        permit: &crate::writer_gate::Permit,
        fault: &mut Fault<'_>,
    ) -> Result<Value> {
        use crate::finalization::Api;
        let operation = crate::writer_gate::operation_key(&self.writer, Api::RecordSet, id);
        let held = refs.iter().chain(deps).cloned().collect::<Vec<_>>();
        let operation_sequence = self.hold_preparation(&operation, &held, fault)?;
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
        let (payload_bytes, tokens) = result_usage(&envelopes, deps, "ValueError")?;
        let record = Record {
            metadata: json!({"record_id":logical,"codec":"record-set.v1","metadata":{},"tokens":0}),
            payload: bytes(&manifest)?.into(),
        };
        // Keep the outer request unresolved through both staging layers and
        // source-holder release; the public raw-write wrapper acknowledges too
        // early for this nested operation.
        let input = InputRecord::from(record);
        let envelope = self.envelope(&input)?;
        let written =
            self.write_prepared(&[input], &[envelope], id, Api::RecordSet, permit, fault)?;
        let result = json!({"manifest":written[0],"digest":logical,"records":refs.len(),
            "payload_bytes":payload_bytes,"tokens":tokens});
        self.writer_registry.as_mut().unwrap().results(
            operation_sequence,
            std::slice::from_ref(&result),
            &written,
            fault,
        )?;
        let handoff = crate::writer_gate::intermediate_handoff(
            &self.config.run_id,
            &self.writer,
            &operation,
            &written,
        )?;
        let consumed = self
            .config
            .catalog(|c| Ok(c.handoff_consumed(&handoff)))?
            .unwrap_or(false);
        if !consumed {
            self.config
                .finalize_publication(&written, &written[0]["segment"], fault)?;
        }
        self.config.stage(std::slice::from_ref(&result), fault)?;
        self.config.catalog(|c| {
            c.consume_once(&self.config, &handoff, &written, fault)?;
            c.release_preparation(&self.config, &[operation], fault)
        })?;
        self.preparations.remove(&crate::writer_gate::operation_key(
            &self.writer,
            Api::RecordSet,
            id,
        ));
        self.writer_registry.as_mut().unwrap().done_owner(
            &crate::writer_gate::operation_key(&self.writer, Api::RecordSet, id),
            fault,
        )?;
        fault("after_publication_receipt")?;
        self.acknowledge_publication();
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
        use crate::finalization::Api;
        self.check_request(Api::Publish, id)?;
        let permit = self.enter_writer(fault)?;
        self.resume_provisional_cleanup(&permit, fault)?;
        self.publish_attempt(groups, records, id, &permit, fault)
    }
    fn publish_attempt(
        &mut self,
        groups: &Value,
        records: Vec<InputRecord>,
        id: &str,
        permit: &crate::writer_gate::Permit,
        fault: &mut Fault<'_>,
    ) -> Result<Vec<Value>> {
        use crate::finalization::Api;
        let groups = groups
            .as_array()
            .ok_or_else(|| Error::new("ValueError", "Expected publication array"))?;
        let operation = crate::writer_gate::operation_key(&self.writer, Api::Publish, id);
        let held = groups
            .iter()
            .flat_map(|g| g["dependencies"].as_array().into_iter().flatten())
            .filter(|d| d.is_object())
            .cloned()
            .collect::<Vec<_>>();
        let operation_sequence = self.hold_preparation(&operation, &held, fault)?;
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
            let (payload_bytes, tokens) = result_usage(&own_envs, &deps, "ValueError")?;
            descriptors.push(json!({"local":members.len(),"digest":logical,"records":ordinals.len(),"payload_bytes":payload_bytes,"tokens":tokens}));
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
        let refs = self.write_prepared(&members, &envs, id, Api::Publish, permit, fault)?;
        let result = descriptors
            .into_iter()
            .map(|mut d| {
                let i = number(&d, "local") as usize;
                d.as_object_mut().unwrap().remove("local");
                d["manifest"] = refs[i].clone();
                d
            })
            .collect::<Vec<_>>();
        self.writer_registry
            .as_mut()
            .unwrap()
            .results(operation_sequence, &result, &[], fault)?;
        self.config
            .finalize_publication(&result, &refs[0]["segment"], fault)?;
        self.config
            .catalog(|c| c.release_preparation(&self.config, &[operation], fault))?;
        self.preparations.remove(&crate::writer_gate::operation_key(
            &self.writer,
            Api::Publish,
            id,
        ));
        self.writer_registry.as_mut().unwrap().done_owner(
            &crate::writer_gate::operation_key(&self.writer, Api::Publish, id),
            fault,
        )?;
        fault("after_publication_receipt")?;
        self.acknowledge_publication();
        Ok(result)
    }
}
pub fn mkdir(path: &Path, fault: &mut Fault<'_>) -> Result<()> {
    mkdir_with_sync(path, |directory| sync_dir(directory, fault))
}

struct DirectoryBarrier<F>(F);
impl<F: FnMut(&Path) -> Result<()>> crate::directory_io::Barrier<&Path> for DirectoryBarrier<F> {
    type Error = Error;
    fn sync(&mut self, path: &&Path) -> Result<()> {
        (self.0)(path)
    }
    fn incomplete() -> Error {
        Error::new("UnsafeRecovery", "Directory barriers did not finish")
    }
}

fn mkdir_with_sync(path: &Path, barrier: impl FnMut(&Path) -> Result<()>) -> Result<()> {
    // Existence does not establish durability: this or another client may
    // have created any ancestor before a failed barrier. Retry the entire
    // chain, including existing names, with no recursive call stack.
    fs::create_dir_all(path)?;
    let canonical = path.canonicalize()?;
    let directories = canonical.ancestors().collect::<Vec<_>>();
    crate::directory_io::sync_all(&directories, &mut DirectoryBarrier(barrier))
}

pub fn sync(file: &File, fault: &mut Fault<'_>) -> Result<()> {
    fault("before_file_sync")?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod directory_tests {
    use super::*;

    #[test]
    fn every_failed_directory_barrier_is_retried_with_all_ancestors() -> Result<()> {
        let private = tempfile::tempdir()?;
        let leaf = private.path().join("pool/raw/partition/writer");
        fs::create_dir_all(&leaf)?; // Existence alone carries no durability proof.
        let canonical = leaf.canonicalize()?;
        let expected: Vec<_> = canonical.ancestors().map(Path::to_path_buf).collect();
        for fail_at in 0..expected.len() {
            let mut failed = Vec::new();
            let result = mkdir_with_sync(&leaf, |p| {
                failed.push(p.to_path_buf());
                if failed.len() == fail_at + 1 {
                    return Err(std::io::Error::from_raw_os_error(5).into());
                }
                Ok(())
            });
            assert_eq!(result.unwrap_err().kind, "StorageUnavailable");
            assert_eq!(failed, expected[..=fail_at]);
            let mut retried = Vec::new();
            mkdir_with_sync(&leaf, |p| {
                retried.push(p.to_path_buf());
                Ok(())
            })?;
            assert_eq!(retried, expected);
        }
        Ok(())
    }

    #[test]
    fn directory_barriers_follow_the_resolved_parent_chain() -> Result<()> {
        let private = tempfile::tempdir()?;
        let target = private.path().join("target");
        fs::create_dir(&target)?;
        std::os::unix::fs::symlink(&target, private.path().join("alias"))?;
        let leaf = private.path().join("alias/child/leaf");
        let mut visited = Vec::new();
        mkdir_with_sync(&leaf, |p| {
            visited.push(p.to_path_buf());
            Ok(())
        })?;
        let canonical = target.join("child/leaf").canonicalize()?;
        assert_eq!(
            visited,
            canonical
                .ancestors()
                .map(Path::to_path_buf)
                .collect::<Vec<_>>()
        );
        Ok(())
    }

    #[test]
    fn an_existing_file_is_not_a_directory() -> Result<()> {
        let private = tempfile::tempdir()?;
        let path = private.path().join("file");
        fs::write(&path, b"keep")?;
        assert!(mkdir_with_sync(&path, |_| panic!("Creation must fail first")).is_err());
        assert_eq!(fs::read(path)?, b"keep");
        Ok(())
    }
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
fn result_usage(envs: &[Value], deps: &[Value], kind: &str) -> Result<(u64, u64)> {
    let total = |field, dependency_field| {
        checked_sum(
            envs.iter()
                .map(|e| number(e, field))
                .chain(deps.iter().map(|d| number(d, dependency_field))),
            kind,
        )
    };
    Ok((
        total("length", "payload_bytes")?,
        total("tokens", "tokens")?,
    ))
}
fn record_refs(r: &Value, count: usize) -> Vec<Value> {
    (0..count)
        .map(|i| json!({"segment":r,"ordinal":i}))
        .collect()
}
// One budget covers header, index, envelope and requested payload/chunks.
// Closures own their descriptors/buffers, so failures drop them before waiting.
fn extent_wait(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}
fn extent_failure(error: Error) -> crate::extent_retry::Failure<Error> {
    if crate::extent_retry::visibility(&error.kind) {
        crate::extent_retry::Failure::Visibility(error)
    } else {
        crate::extent_retry::Failure::Permanent(error)
    }
}
fn extent_retry_error(error: crate::extent_retry::Error<Error>) -> Error {
    match error {
        crate::extent_retry::Error::Permanent(error) => error,
        crate::extent_retry::Error::Exhausted(mut error) => {
            crate::extent_retry::exhausted(&mut error.kind);
            error
        }
    }
}
fn index_selection_error(error: crate::record_index::Error) -> Error {
    match error {
        crate::record_index::Error::Records => {
            Error::new("InvalidReference", "Missing array records")
        }
        crate::record_index::Error::Ordinal => Error::new("InvalidReference", "Invalid ordinal"),
        crate::record_index::Error::Outside => {
            Error::new("InvalidReference", "Ordinal outside segment")
        }
    }
}
impl crate::record_index::Source for &Config {
    type File = File;
    type Error = Error;
    fn inspect(
        &mut self,
        reference: &Value,
    ) -> std::result::Result<(File, Value), crate::extent_retry::Failure<Error>> {
        Config::inspect_opened(self, reference, false).map_err(extent_failure)
    }
}
struct ExtentLog<'a>(&'a Value);
impl crate::extent_retry::Log<Error> for ExtentLog<'_> {
    fn recovered(&mut self, retries: usize) {
        eprintln!(
            "straw extent visible after {retries} retries; path={}, offset={}",
            self.0["path"],
            number(self.0, "offset")
        );
    }
    fn retry(&mut self, retry: usize, limit: usize, delay: u64, error: &Error) {
        eprintln!(
            "straw extent visibility retry {retry}/{limit} after {delay}ms; path={}, offset={}: CorruptData: {}",
            self.0["path"],
            number(self.0, "offset"),
            error.message
        );
    }
}
struct ExtentWait<W: FnMut(u64)>(W);
impl<W: FnMut(u64)> crate::extent_retry::Wait for ExtentWait<W> {
    fn wait(&mut self, delay: u64) {
        (self.0)(delay);
    }
}
fn extent_observer<W: FnMut(u64)>(
    segment: &Value,
    wait: W,
) -> crate::extent_retry::Observe<ExtentLog<'_>, ExtentWait<W>> {
    crate::extent_retry::Observe::new(ExtentLog(segment), ExtentWait(wait))
}
struct ExtentCall<F>(F);
impl<T, F: FnMut() -> Result<T>> crate::extent_retry::CallableModel for ExtentCall<F> {
    type Output = T;
    type Error = Error;
}
impl<T, F: FnMut() -> Result<T>> crate::extent_retry::Callable for ExtentCall<F> {
    fn read(&mut self) -> std::result::Result<T, crate::extent_retry::Failure<Error>> {
        (self.0)().map_err(extent_failure)
    }
}
fn retry_extent<T>(segment: &Value, read: impl FnMut() -> Result<T>) -> Result<T> {
    retry_extent_with_wait(segment, read, extent_wait)
}
fn retry_extent_with_wait<T>(
    segment: &Value,
    read: impl FnMut() -> Result<T>,
    wait: impl FnMut(u64),
) -> Result<T> {
    crate::extent_retry::run(
        &mut crate::extent_retry::Calls::new(ExtentCall(read)),
        &mut extent_observer(segment, wait),
        &EXTENT_VISIBILITY_DELAYS_MS,
    )
    .map_err(extent_retry_error)
}
fn visibility_io(e: std::io::Error) -> Error {
    if e.kind() == std::io::ErrorKind::UnexpectedEof {
        Error::new(EXTENT_VISIBILITY, e.to_string())
    } else {
        e.into()
    }
}
fn visible_exact(f: &mut File, n: usize) -> Result<Vec<u8>> {
    let mut v = vec![0; n];
    f.read_exact(&mut v).map_err(visibility_io)?;
    Ok(v)
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

pub(crate) fn relative_storage_path(root: &Path, relative: &str) -> Result<PathBuf> {
    if !crate::namespace_identity::valid(relative.as_bytes(), Path::new(relative).is_absolute()) {
        return fail("InvalidReference", "Path escapes run root");
    }
    Ok(root.join(relative))
}

#[doc(hidden)]
pub struct NamespaceParts<'a>(std::str::Split<'a, char>);
impl crate::namespace_identity::Parts for NamespaceParts<'_> {
    fn next(&mut self) -> Option<&str> {
        self.0.next()
    }
}

#[doc(hidden)]
pub struct NamespaceBackend {
    root: PathBuf,
}
impl crate::namespace_step::Model for NamespaceBackend {
    type Error = Error;
}
impl crate::namespace_step::Primitives for NamespaceBackend {
    fn root(&self, _prefix: &String) -> PathBuf {
        self.root.to_path_buf()
    }
    fn cached(&self, _prefix: &String, identity: &String) -> PathBuf {
        self.root.join(identity)
    }
    fn child(&self, base: &PathBuf, component: &str, _prefix: &String) -> PathBuf {
        base.join(component)
    }
    fn child_link(
        &mut self,
        _base: &PathBuf,
        candidate: &PathBuf,
        _prefix: &String,
    ) -> std::io::Result<PathBuf> {
        fs::read_link(candidate)
    }
    fn join(&self, base: &PathBuf, target: PathBuf) -> PathBuf {
        base.join(target)
    }
    fn relative(&self, path: &PathBuf, _prefix: &String) -> Result<String> {
        path.strip_prefix(&self.root)
            .ok()
            .and_then(Path::to_str)
            .map(str::to_owned)
            .ok_or_else(|| Error::new("InvalidReference", "Namespace identity escapes run root"))
    }
    fn io_error(&self, error: std::io::Error) -> Error {
        error.into()
    }
    fn target_error(&self, error: crate::namespace_target::Error<std::io::Error>) -> Error {
        use crate::namespace_target::Error as TargetError;
        match error {
            TargetError::Io(error) => error.into(),
            TargetError::Links => Error::new("InvalidReference", "Too many namespace symlinks"),
            TargetError::Root => Error::new("InvalidReference", "Invalid namespace root"),
            TargetError::Path => Error::new("InvalidReference", "Invalid namespace path"),
        }
    }
}

#[doc(hidden)]
pub struct NamespaceFactory;
impl crate::namespace_identity::Factory<Error> for NamespaceFactory {
    type Iter<'a> = NamespaceParts<'a>;
    fn parts<'a>(&self, name: &'a str) -> Self::Iter<'a> {
        NamespaceParts(name.split('/'))
    }
    fn absolute(&self, name: &str) -> bool {
        Path::new(name).is_absolute()
    }
    fn identity_error(&self, error: crate::namespace_identity::Error<Error>) -> Error {
        match error {
            crate::namespace_identity::Error::Invalid => {
                Error::new("InvalidReference", "Path escapes run root")
            }
            crate::namespace_identity::Error::Pool => Error::new(
                "InvalidReference",
                "Storage identity names the pool directory",
            ),
            crate::namespace_identity::Error::Changed => {
                Error::new("UnsafeRecovery", "Storage namespace identity changed")
            }
            crate::namespace_identity::Error::Read(error) => error,
        }
    }
    fn catalog_error(&self, unsupported: bool) -> Error {
        if unsupported {
            Error::new("CorruptData", "Unsupported catalog path encoding")
        } else {
            Error::new("InvalidReference", "Path escapes run root")
        }
    }
}
pub(crate) type NamespaceSource = crate::catalog_namespace::PathSource<
    crate::namespace_step::Backend<NamespaceBackend>,
    NamespaceFactory,
>;
pub(crate) type NamespaceState = crate::catalog_namespace::Adapter<NamespaceSource>;
pub(crate) fn namespace_source(root: PathBuf) -> NamespaceSource {
    crate::catalog_namespace::PathSource::new(
        crate::namespace_step::Backend::new(NamespaceBackend { root }),
        NamespaceFactory,
    )
}
#[cfg(test)]
pub(crate) fn namespace(root: PathBuf) -> NamespaceState {
    crate::catalog_namespace::Adapter::new(namespace_source(root))
}

impl crate::namespace_target::Model for NamespaceBackend {
    type Context = ();
    type Path = PathBuf;
    type Part = std::ffi::OsString;
    type IoError = std::io::Error;
}
impl crate::namespace_target::Backend for NamespaceBackend {
    fn canonicalize(&mut self, path: &PathBuf) -> std::io::Result<PathBuf> {
        path.canonicalize()
    }
    fn read_link(&mut self, path: &PathBuf) -> std::io::Result<PathBuf> {
        fs::read_link(path)
    }
    fn classify(&self, error: &std::io::Error) -> crate::namespace_target::ErrorKind {
        use crate::namespace_target::ErrorKind;
        match error.kind() {
            std::io::ErrorKind::NotFound => ErrorKind::Missing,
            std::io::ErrorKind::InvalidInput => ErrorKind::NotLink,
            _ => ErrorKind::Other,
        }
    }
    fn is_absolute(&self, path: &PathBuf) -> bool {
        path.is_absolute()
    }
    fn join_parent(&self, path: &PathBuf, target: PathBuf) -> Option<PathBuf> {
        path.parent().map(|parent| parent.join(target))
    }
    fn last(&self, path: &PathBuf) -> Option<std::ffi::OsString> {
        path.components()
            .next_back()
            .map(|component| component.as_os_str().to_owned())
    }
    fn pop(&self, path: &mut PathBuf) -> bool {
        path.pop()
    }
    fn part_kind(&self, part: &std::ffi::OsString) -> crate::namespace_target::PartKind {
        use crate::namespace_target::PartKind;
        if part == ".." {
            PartKind::Parent
        } else if part == "." {
            PartKind::Current
        } else {
            PartKind::Other
        }
    }
    fn push(&self, path: &mut PathBuf, part: std::ffi::OsString) {
        path.push(part);
    }
}

pub fn storage_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = relative_storage_path(root, relative)?;
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

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn namespace_identity_survives_missing_targets_and_rejects_escape() -> Result<()> {
        let private = tempfile::tempdir()?;
        let root = private.path().join("pool");
        fs::create_dir_all(root.join("real/child"))?;
        let mut state = namespace(root.canonicalize()?);
        let mut identity = |path: &str| state.identity(path);
        std::os::unix::fs::symlink("real", root.join("alias"))?;
        std::os::unix::fs::symlink("alias/new/data.pack", root.join("leaf.pack"))?;
        assert_eq!(identity("alias/new/data.pack")?, "real/new/data.pack");
        assert_eq!(identity("leaf.pack")?, "real/new/data.pack");
        fs::create_dir_all(root.join("real/new"))?;
        fs::write(root.join("real/new/data.pack"), b"bytes")?;
        assert_eq!(identity("leaf.pack")?, "real/new/data.pack");
        fs::remove_file(root.join("real/new/data.pack"))?;
        assert_eq!(identity("leaf.pack")?, "real/new/data.pack");
        std::os::unix::fs::symlink("real/child/../gone.pack", root.join("parent.pack"))?;
        assert_eq!(identity("parent.pack")?, "real/gone.pack");
        std::os::unix::fs::symlink("../outside/missing.pack", root.join("escape.pack"))?;
        assert_eq!(
            identity("escape.pack").unwrap_err().kind,
            "InvalidReference"
        );
        assert!(!private.path().join("outside").exists());
        std::os::unix::fs::symlink("cycle.pack", root.join("cycle.pack"))?;
        assert!(identity("cycle.pack").is_err());
        Ok(())
    }

    #[test]
    fn missing_symlink_suffix_restores_parents_and_pool_aliases() -> Result<()> {
        let private = tempfile::tempdir()?;
        let root = private.path().join("pool");
        fs::create_dir_all(root.join("real"))?;
        let root = root.canonicalize()?;
        std::os::unix::fs::symlink(
            "missing/deeper/../../real/data.pack",
            root.join("parent.pack"),
        )?;
        std::os::unix::fs::symlink(".", root.join("self"))?;
        std::os::unix::fs::symlink("parent.pack", root.join("indirect.pack"))?;
        let mut state = namespace(root.clone());
        assert_eq!(state.identity("parent.pack")?, "real/data.pack");
        assert_eq!(state.identity("indirect.pack")?, "real/data.pack");
        assert_eq!(state.identity("self").unwrap_err().kind, "InvalidReference");
        assert_eq!(state.identity("self/real/数据.pack")?, "real/数据.pack");
        for invalid in [
            "",
            "/real/data.pack",
            "real//data.pack",
            "real/./data.pack",
            "real/../data.pack",
            "real/data.pack/",
            "real\\data.pack",
        ] {
            assert_eq!(
                state.identity(invalid).unwrap_err().kind,
                "InvalidReference"
            );
        }
        fs::write(root.join("not-directory"), b"data")?;
        std::os::unix::fs::symlink("not-directory/child", root.join("invalid.pack"))?;
        assert!(state.identity("invalid.pack").is_err());
        Ok(())
    }
}

#[cfg(test)]
mod visibility_tests {
    use super::*;

    #[test]
    fn verification_checks_empty_payload_buffers_tail_and_short_reads() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("payload");
        let buffer = 1024 * 1024;
        for length in [0, 1, buffer - 1, buffer, buffer + 1, 2 * buffer + 7] {
            let payload = vec![b'x'; length];
            let env = json!({"length":length,"checksum":hash(&payload)});
            fs::write(&path, &payload)?;
            Config::verify_frame(File::open(&path)?, env.clone())?;
            if length == 0 {
                let wrong = json!({"length":0,"checksum":hash(b"x")});
                assert_eq!(
                    Config::verify_frame(File::open(&path)?, wrong)
                        .unwrap_err()
                        .kind,
                    EXTENT_VISIBILITY
                );
                continue;
            }
            let mut positions = vec![0, length - 1];
            if length > buffer {
                positions.extend([buffer - 1, buffer]);
            }
            positions.sort_unstable();
            positions.dedup();
            for position in positions {
                let mut f = OpenOptions::new().write(true).open(&path)?;
                f.seek(SeekFrom::Start(position as u64))?;
                f.write_all(b"y")?;
                let error = Config::verify_frame(File::open(&path)?, env.clone()).unwrap_err();
                assert_eq!(error.kind, EXTENT_VISIBILITY);
                assert!(error.message.contains("payload checksum"));
                f.seek(SeekFrom::Start(position as u64))?;
                f.write_all(b"x")?;
            }
            OpenOptions::new()
                .write(true)
                .open(&path)?
                .set_len((length - 1) as u64)?;
            assert_eq!(
                Config::verify_frame(File::open(&path)?, env)
                    .unwrap_err()
                    .kind,
                EXTENT_VISIBILITY
            );
        }
        Ok(())
    }

    #[test]
    fn inspected_descriptor_supplies_frame_after_name_is_unavailable() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let config = Config::new(
            dir.path().to_str().unwrap(),
            "opened",
            vec!["bytes.v1".into()],
            1024,
            1024,
            4096,
        )?;
        let mut writer = Store::new(config.clone());
        let record = Record {
            metadata: json!({"record_id":"r","codec":"bytes.v1","metadata":{},"tokens":0}),
            payload: b"bound descriptor".to_vec().into(),
        };
        let refs = writer.write(&[record], "source", &mut |_| Ok(()))?;
        let reference = &refs[0];
        let path = config.path(field(&reference["segment"], "path")?)?;
        let (file, index) = config.inspect_opened(&reference["segment"], false)?;
        // A test-only namespace change makes an accidental reopen observable.
        // It does not extend the production stable-namespace contract.
        fs::rename(&path, path.with_extension("held"))?;
        let (file, env) = config.frame_with_index(reference, &index, Some(file))?;
        assert_eq!(
            Config::read_frame(file, env)?.payload.as_ref(),
            b"bound descriptor"
        );
        assert!(config.frame_with_index(reference, &index, None).is_err());
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failed_read_releases_its_view_before_wait_and_opens_the_next_view() -> Result<()> {
        for target in [0, 4096] {
            for session in [0, 1, 2] {
                // ordinary, cold session, warm session
                for index_gap in [false, true] {
                    let dir = tempfile::tempdir()?;
                    let config = Config::new(
                        dir.path().to_str().unwrap(),
                        "reopen",
                        vec!["bytes.v1".into()],
                        1024,
                        1024,
                        target,
                    )?;
                    let mut writer = Store::new(config.clone());
                    let record = Record {
                        metadata: json!({"record_id":"r","codec":"bytes.v1","metadata":{},"tokens":0}),
                        payload: b"next complete view".to_vec().into(),
                    };
                    writer.write(std::slice::from_ref(&record), "prefix", &mut |_| Ok(()))?;
                    let refs = writer.write(&[record], "source", &mut |_| Ok(()))?;
                    let reference = &refs[0];
                    let segment = &reference["segment"];
                    let path = config.path(field(segment, "path")?)?;
                    let good = fs::read(&path)?;
                    let index = config.inspect(segment, false)?;
                    let mut reader = Reader::new(config.clone());
                    if session == 2 {
                        reader.index(segment)?;
                    }
                    let footer =
                        (number(segment, "offset") + number(segment, "size") - TRAILER) as usize;
                    let index_len =
                        u64::from_le_bytes(good[footer..footer + 8].try_into().unwrap()) as usize;
                    let frame = (number(segment, "offset") + number(&index["records"][0], "offset"))
                        as usize;
                    let metadata =
                        u32::from_le_bytes(good[frame..frame + 4].try_into().unwrap()) as usize;
                    // Warm sessions legitimately skip the already authenticated
                    // index, so use a payload gap in that combination.
                    let position = if index_gap && session != 2 {
                        footer - index_len
                    } else {
                        frame + 12 + metadata
                    };
                    let mut stale = good.clone();
                    stale[position] ^= 1;
                    fs::write(&path, stale)?;
                    let mut waits = 0;
                    let result = retry_extent_with_wait(
                        segment,
                        || {
                            let (file, env) = if session == 0 {
                                config.frame(reference)?
                            } else {
                                reader.frame(reference)?
                            };
                            Config::read_frame(file, env)
                        },
                        |_| {
                            waits += 1;
                            assert_eq!(waits, 1);
                            let old_view = path.with_extension("previous-view");
                            fs::rename(&path, &old_view).unwrap();
                            // No descriptor may retain the failed view across
                            // the retry wait, including the session cache.
                            for entry in fs::read_dir("/proc/self/fd").unwrap().flatten() {
                                assert_ne!(
                                    fs::read_link(entry.path()).ok().as_ref(),
                                    Some(&old_view)
                                );
                            }
                            // Model per-open observations with different inodes:
                            // an old descriptor would continue seeing stale bytes.
                            fs::write(&path, &good).unwrap();
                        },
                    )?;
                    assert_eq!(waits, 1);
                    assert_eq!(result.payload.as_ref(), b"next complete view");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn length_and_regions_become_visible_independently_with_one_budget() -> Result<()> {
        use std::cell::{Cell, RefCell};
        for target in [0, 4096] {
            for cached_session in [false, true] {
                let dir = tempfile::tempdir()?;
                let config = Config::new(
                    dir.path().to_str().unwrap(),
                    "independent-views",
                    vec!["bytes.v1".into()],
                    1024,
                    1024,
                    target,
                )?;
                let mut writer = Store::new(config.clone());
                let record = Record {
                    metadata: json!({"record_id":"r","codec":"bytes.v1","metadata":{},"tokens":0}),
                    payload: b"eventually visible payload".to_vec().into(),
                };
                writer.write(std::slice::from_ref(&record), "prefix", &mut |_| Ok(()))?;
                let references = writer.write(&[record], "source", &mut |_| Ok(()))?;
                let reference = &references[0];
                let segment = &reference["segment"];
                let path = config.path(field(segment, "path")?)?;
                let index = config.inspect(segment, false)?;
                let committed = fs::read(&path)?;
                let offset = number(segment, "offset") as usize;
                let end = offset + number(segment, "size") as usize;
                let footer = end - TRAILER as usize;
                let index_len =
                    u64::from_le_bytes(committed[footer..footer + 8].try_into().unwrap()) as usize;
                let frame = offset + number(&index["records"][0], "offset") as usize;
                let metadata_len =
                    u32::from_le_bytes(committed[frame..frame + 4].try_into().unwrap()) as usize;
                let mut views = vec![committed[..end - 1].to_vec()];
                for position in [
                    offset,
                    footer + 72,
                    footer - index_len,
                    frame + 12 + metadata_len,
                ] {
                    let mut view = committed.clone();
                    assert_ne!(view[position], 0);
                    view[position] = 0;
                    views.push(view);
                }
                views.push(committed);
                fs::write(&path, &views[0])?;
                let waits = Cell::new(0usize);
                let attempts = Cell::new(0usize);
                let errors = RefCell::new(Vec::new());
                let mut reader = Reader::new(config.clone());
                let result = retry_extent_with_wait(
                    segment,
                    || {
                        attempts.set(attempts.get() + 1);
                        let outcome = (|| {
                            let (file, env) = if cached_session {
                                reader.frame(reference)?
                            } else {
                                config.frame(reference)?
                            };
                            Config::read_frame(file, env)
                        })();
                        if let Err(error) = &outcome {
                            assert_eq!(error.kind, EXTENT_VISIBILITY);
                            errors.borrow_mut().push(error.message.clone());
                        }
                        outcome
                    },
                    |delay| {
                        let next = waits.get() + 1;
                        assert_eq!(delay, EXTENT_VISIBILITY_DELAYS_MS[next - 1]);
                        waits.set(next);
                        fs::write(&path, &views[next]).unwrap();
                    },
                )?;
                assert_eq!(result.payload.as_ref(), b"eventually visible payload");
                assert_eq!(waits.get(), EXTENT_VISIBILITY_DELAYS_MS.len());
                assert_eq!(attempts.get(), EXTENT_VISIBILITY_DELAYS_MS.len() + 1);
                let errors = errors.into_inner();
                assert_eq!(errors.len(), 5);
                for (message, expected) in errors.iter().zip([
                    "Segment size mismatch",
                    "Invalid segment header",
                    "Invalid segment footer/checksum",
                    "Segment index checksum mismatch",
                    "Record payload checksum mismatch",
                ]) {
                    assert!(message.contains(expected), "{message}");
                }
            }
        }
        Ok(())
    }

    // Replay a shrinking counterexample through real native file reads without
    // timing/thread races: every wait restores one region and hides the next.
    #[test]
    fn one_retry_budget_covers_header_index_envelope_and_payload() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let config = Config::new(
            dir.path().to_str().unwrap(),
            "visibility",
            vec!["bytes.v1".into()],
            1024,
            1024,
            4096,
        )?;
        let mut writer = Store::new(config.clone());
        let record = Record {
            metadata: json!({"record_id":"r","codec":"bytes.v1","metadata":{},"tokens":0}),
            payload: b"committed payload".to_vec().into(),
        };
        writer.write(std::slice::from_ref(&record), "prefix", &mut |_| Ok(()))?;
        let refs = writer.write(&[record], "r", &mut |_| Ok(()))?;
        let reference = &refs[0];
        let segment = &reference["segment"];
        let index = config.inspect(segment, false)?;
        let path = config.path(field(segment, "path")?)?;
        let committed = fs::read(&path)?;
        let offset = number(segment, "offset") as usize;
        assert!(offset > 0);
        let footer = offset + number(segment, "size") as usize - TRAILER as usize;
        let index_len =
            u64::from_le_bytes(committed[footer..footer + 8].try_into().unwrap()) as usize;
        let frame = offset + number(&index["records"][0], "offset") as usize;
        let meta_len = u32::from_le_bytes(committed[frame..frame + 4].try_into().unwrap()) as usize;
        let regions = [
            offset,
            footer + 72,
            footer - index_len,
            frame,
            frame + 12,
            frame + 12 + meta_len,
        ];
        let hide = |position: usize| {
            let mut stale = committed.clone();
            assert_ne!(stale[position], 0);
            stale[position] = 0;
            fs::write(&path, stale).unwrap();
        };
        for (verify_only, succeed) in [(false, false), (false, true), (true, false), (true, true)] {
            hide(regions[0]);
            let mut waits = Vec::new();
            let mut opens = 0;
            let outcome = retry_extent_with_wait(
                segment,
                || {
                    opens += 1;
                    let (f, env) = config.frame(reference)?;
                    if verify_only {
                        Config::verify_frame(f, env).map(|()| None)
                    } else {
                        Config::read_frame(f, env).map(Some)
                    }
                },
                |delay| {
                    waits.push(delay);
                    if succeed && waits.len() == EXTENT_VISIBILITY_DELAYS_MS.len() {
                        fs::write(&path, &committed).unwrap();
                    } else {
                        hide(regions[waits.len()]);
                    }
                },
            );
            assert_eq!(waits, EXTENT_VISIBILITY_DELAYS_MS);
            assert_eq!(opens, 6);
            if succeed {
                let record = outcome?;
                assert_eq!(record.is_none(), verify_only);
                if let Some(record) = record {
                    assert_eq!(record.payload.as_ref(), b"committed payload");
                }
            } else {
                let error = outcome.err().unwrap();
                assert_eq!(error.kind, "CorruptData");
                assert!(error.message.contains("payload checksum"));
            }
        }
        // A nonzero wrong header and wrong identity must not consume retries.
        let mut bad = committed;
        bad[offset] = 255;
        fs::write(&path, bad)?;
        let error = retry_extent_with_wait(
            segment,
            || config.inspect_once(segment, false),
            |_| panic!("retry"),
        )
        .unwrap_err();
        assert_eq!(error.kind, "CorruptData");
        let mut wrong = segment.clone();
        wrong["run_id"] = json!("other");
        let error = retry_extent_with_wait(
            &wrong,
            || config.inspect_once(&wrong, false),
            |_| panic!("retry"),
        )
        .unwrap_err();
        assert_eq!(error.kind, "InvalidReference");
        Ok(())
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    fn config(root: &Path) -> Result<Config> {
        Config::new(
            root.to_str().unwrap(),
            "discovery",
            vec![],
            1024,
            1024,
            1024,
        )
    }

    #[test]
    fn discovery_errors_are_not_legacy_and_required_mode_is_shared() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cfg = config(dir.path())?;
        assert!(!cfg.gc_enabled_with_probe(|| Err(std::io::ErrorKind::NotFound.into()))?);
        let error = cfg
            .gc_enabled_with_probe(|| Err(std::io::Error::from_raw_os_error(5)))
            .unwrap_err();
        assert_eq!(error.kind, "StorageUnavailable");
        assert!(!cfg.gc_enabled_with_probe(|| Err(std::io::ErrorKind::NotFound.into()))?);
        let clone = cfg.clone();
        assert!(cfg.gc_enabled_with_probe(|| Ok(()))?);
        assert!(clone.gc_enabled_with_probe(|| panic!("Known GC must not be rediscovered"))?);
        assert!(cfg.gc_enabled_with_probe(|| panic!("Known GC must not be rediscovered"))?);
        Ok(())
    }

    #[test]
    fn discovery_rechecks_other_clones_after_a_probe() -> Result<()> {
        for absent in [false, true] {
            let dir = tempfile::tempdir()?;
            let cfg = config(dir.path())?;
            let clone = cfg.clone();
            assert!(cfg.gc_enabled_with_probe(|| {
                assert!(clone.gc_enabled_with_probe(|| Ok(())).unwrap());
                Err(if absent {
                    std::io::ErrorKind::NotFound.into()
                } else {
                    std::io::Error::from_raw_os_error(5)
                })
            })?);
        }
        Ok(())
    }

    #[test]
    fn discovered_catalog_disappearance_is_not_legacy_or_recreation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut creator = config(dir.path())?;
        creator.online_gc = true;
        creator.catalog(|_| Ok(()))?;
        let cfg = config(dir.path())?;
        assert!(cfg.gc_enabled()?);
        let catalog = dir.path().join("storage.log");
        let moved = dir.path().join("storage.saved");
        fs::rename(&catalog, &moved)?;
        assert!(cfg.gc_enabled()?);
        assert_eq!(
            cfg.catalog(|_| Ok(())).unwrap_err().kind,
            "StorageUnavailable"
        );
        assert!(
            !catalog.exists(),
            "Discovery must not recreate a missing catalog"
        );
        fs::rename(&moved, &catalog)?;
        assert!(cfg.catalog(|_| Ok(()))?.is_some());
        fs::rename(&catalog, &moved)?;
        assert_eq!(
            cfg.catalog(|_| Ok(())).unwrap_err().kind,
            "StorageUnavailable"
        );
        assert!(!catalog.exists());
        fs::rename(&moved, &catalog)?;
        assert!(cfg.catalog(|_| Ok(()))?.is_some());
        Ok(())
    }
}
