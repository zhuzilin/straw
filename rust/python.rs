//! Python bindings are conversion and exception boundaries; all durable state
//! and filesystem operations execute in Rust with the GIL released.
use crate::coordinator::Coordinator;
use crate::journal::Journal;
use crate::payload::{InputRecord, Payload};
use crate::store::{Config, GraphProof, Reader, Store};
use crate::{Error, Fault, Result};
use pyo3::{
    buffer::PyBuffer,
    exceptions::{PyOSError, PyRuntimeError},
    prelude::*,
    types::{PyBytes, PyModule},
};
use serde_json::{Value, json};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

struct ValidationScopeState {
    graph: Mutex<Option<GraphProof>>,
    closed: AtomicBool,
}
impl ValidationScopeState {
    fn invalidate(&self) {
        self.closed.store(true, Ordering::Release);
        if let Ok(mut graph) = self.graph.try_lock() {
            *graph = None;
        }
    }
}
#[pyclass]
struct NativeValidationScope {
    state: Arc<ValidationScopeState>,
    pid: u32,
}
impl NativeValidationScope {
    fn checked(&self) -> PyResult<Arc<ValidationScopeState>> {
        if self.pid != std::process::id() {
            return Err(PyRuntimeError::new_err(
                "Validation scope cannot be reused after fork",
            ));
        }
        Ok(self.state.clone())
    }
}
#[pymethods]
impl NativeValidationScope {
    fn close(&self) -> PyResult<()> {
        self.checked()?.invalidate();
        Ok(())
    }
    fn stats(&self) -> PyResult<String> {
        let state = self.checked()?;
        let guard = state
            .graph
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("Validation scope is busy"))?;
        let graph = guard
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Validation scope is closed"))?;
        Ok(json!({"nodes":graph.node_count(),"checked_nodes":graph.checked_nodes,"reused_nodes":graph.reused_nodes,"published_nodes":graph.published_nodes}).to_string())
    }
}
fn scoped<T>(
    state: Option<&Arc<ValidationScopeState>>,
    call: impl FnOnce(Option<&mut GraphProof>) -> Result<T>,
) -> Result<T> {
    let Some(state) = state else {
        return call(None);
    };
    if state.closed.load(Ordering::Acquire) {
        return Err(Error::new("RuntimeError", "Validation scope is closed"));
    }
    let mut guard = match state.graph.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            state.invalidate();
            return Err(Error::new(
                "RuntimeError",
                "Validation scope is already in use",
            ));
        }
    };
    if state.closed.load(Ordering::Acquire) {
        *guard = None;
        return Err(Error::new("RuntimeError", "Validation scope is closed"));
    }
    let result = call(Some(guard.as_mut().ok_or_else(|| {
        Error::new("RuntimeError", "Validation scope is closed")
    })?));
    if result.is_err() || state.closed.load(Ordering::Acquire) {
        state.closed.store(true, Ordering::Release);
        *guard = None;
    }
    result
}

fn parse(text: &str) -> Result<Value> {
    Ok(serde_json::from_str(text)?)
}
fn config(text: &str) -> Result<Config> {
    let v = parse(text)?;
    let mut config = Config::new(
        crate::field(&v, "root")?,
        crate::field(&v, "run_id")?,
        serde_json::from_value(v["codecs"].clone())?,
        crate::number(&v, "max_record_bytes"),
        crate::number(&v, "max_buffer_bytes"),
        crate::number(&v, "segment_target_bytes"),
    )?;
    config.online_gc = v["online_gc"].as_bool().unwrap_or(false);
    Ok(config)
}
fn exception(py: Python<'_>, error: Error) -> PyErr {
    let module = if matches!(
        error.kind.as_str(),
        "ValueError" | "TypeError" | "RuntimeError" | "FileExistsError"
    ) {
        "builtins"
    } else {
        "straw.errors"
    };
    match py
        .import(module)
        .and_then(|m| m.getattr(error.kind.as_str()))
        .and_then(|cls| cls.call1((error.message.clone(),)))
    {
        Ok(value) => PyErr::from_value(value),
        Err(_) => PyRuntimeError::new_err(error.to_string()),
    }
}
fn run<T: Send>(
    py: Python<'_>,
    callback: &Py<PyAny>,
    call: impl FnOnce(&mut Fault<'_>) -> Result<T> + Send,
) -> PyResult<T> {
    let (result, original) = py.allow_threads(|| {
        let mut original = None;
        let mut fault = |phase: &str| -> Result<()> {
            Python::with_gil(|py| {
                if let Err(e) = callback.call1(py, (phase,)) {
                    if phase != "before_complete_reply" && e.is_instance_of::<PyOSError>(py) {
                        return Err(Error::new("StorageUnavailable", e.to_string()));
                    }
                    let message = e.to_string();
                    original = Some(e);
                    return Err(Error::new("Hook", message));
                }
                Ok(())
            })
        };
        let result = call(&mut fault);
        (result, original)
    });
    match result {
        Ok(value) => Ok(value),
        Err(error) => Err(original.unwrap_or_else(|| exception(py, error))),
    }
}
// PyBuffer pins the exporting allocation. Only copy its cells while holding the
// GIL, then release the GIL for checksumming and file I/O. Never construct a Rust
// shared slice over mutable Python memory across a GIL release.
struct PythonPayload(PyBuffer<u8>);
impl PythonPayload {
    fn copy_chunk(&self, start: usize, size: usize) -> Result<Vec<u8>> {
        Python::with_gil(|py| {
            let source = self
                .0
                .as_slice(py)
                .ok_or_else(|| Error::new("ValueError", "Input buffer must be C-contiguous"))?;
            Ok(source[start..(start + size).min(self.len())]
                .iter()
                .map(|cell| cell.get())
                .collect())
        })
    }
}
impl Payload for PythonPayload {
    fn len(&self) -> usize {
        self.0.len_bytes()
    }
    fn chunks(&self, size: usize, visit: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> {
        if self.len() <= size.saturating_mul(2) || size < 4096 {
            for start in (0..self.len()).step_by(size) {
                visit(&self.copy_chunk(start, size)?)?;
            }
            return Ok(());
        }
        // A rendezvous channel permits one producer buffer and one consumer
        // buffer. Copy the next chunk while the current one is hashed/written.
        // The consumer alone appends, so byte order and publication stay atomic.
        std::thread::scope(|scope| {
            let (sender, receiver) = std::sync::mpsc::sync_channel(0);
            let producer = scope.spawn(move || -> Result<()> {
                for start in (0..self.len()).step_by(size) {
                    if sender.send(self.copy_chunk(start, size)?).is_err() {
                        break;
                    }
                }
                Ok(())
            });
            let result = (|| {
                for chunk in &receiver {
                    visit(&chunk)?;
                }
                Ok(())
            })();
            drop(receiver); // Unblock producer on I/O error or fault injection.
            let copied = producer
                .join()
                .map_err(|_| Error::new("RuntimeError", "Payload copy worker panicked"))?;
            result.and(copied)
        })
    }
}

fn records(
    py: Python<'_>,
    _config: &Config,
    metadata: &str,
    payloads: Vec<Py<PyAny>>,
) -> PyResult<Vec<InputRecord>> {
    let entries: Vec<Value> =
        serde_json::from_str(metadata).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    if entries.len() != payloads.len() {
        return Err(PyRuntimeError::new_err("Metadata/payload count mismatch"));
    }
    entries
        .into_iter()
        .zip(payloads)
        .map(|(metadata, payload)| {
            let buffer = PyBuffer::<u8>::get(payload.bind(py))?;
            if !buffer.is_c_contiguous() {
                return Err(exception(
                    py,
                    Error::new("ValueError", "Input buffer must be C-contiguous"),
                ));
            }
            Ok(InputRecord {
                metadata,
                payload: Arc::new(PythonPayload(buffer)),
            })
        })
        .collect()
}
#[pyclass]
struct NativeReader {
    reader: Mutex<Option<Reader>>,
    pid: u32,
}
impl NativeReader {
    fn check_process(&self) -> PyResult<()> {
        // Check before taking the mutex: after fork its owner thread may no
        // longer exist in the child, so even closing must not acquire it.
        if self.pid != std::process::id() {
            return Err(PyRuntimeError::new_err(
                "Read session cannot be reused after fork; open a new session in this process",
            ));
        }
        Ok(())
    }
    fn with_reader<T: Send>(
        &self,
        py: Python<'_>,
        operation: impl FnOnce(&mut Reader) -> Result<T> + Send,
    ) -> PyResult<T> {
        self.check_process()?;
        py.allow_threads(|| {
            let mut guard = self
                .reader
                .lock()
                .map_err(|_| Error::new("RuntimeError", "Reader mutex poisoned"))?;
            let reader = guard
                .as_mut()
                .ok_or_else(|| Error::new("RuntimeError", "Reader is closed"))?;
            operation(reader)
        })
        .map_err(|e| exception(py, e))
    }
}
#[pymethods]
impl NativeReader {
    fn manifest(&self, py: Python<'_>, reference: &str) -> PyResult<String> {
        self.with_reader(py, |r| {
            Ok(serde_json::to_string(&r.manifest(&parse(reference)?)?)?)
        })
    }
    #[pyo3(signature = (reference, task=None, attempt=None))]
    fn validate(
        &self,
        py: Python<'_>,
        reference: &str,
        task: Option<String>,
        attempt: Option<String>,
    ) -> PyResult<String> {
        self.with_reader(py, |r| {
            Ok(serde_json::to_string(&r.validate(
                &parse(reference)?,
                task.as_deref(),
                attempt.as_deref(),
            )?)?)
        })
    }
    fn envelope(&self, py: Python<'_>, reference: &str) -> PyResult<String> {
        self.with_reader(py, |r| {
            Ok(serde_json::to_string(&r.envelope(&parse(reference)?)?)?)
        })
    }
    #[pyo3(signature = (reference, codec=None))]
    fn member_envelopes(
        &self,
        py: Python<'_>,
        reference: &str,
        codec: Option<String>,
    ) -> PyResult<String> {
        self.with_reader(py, |reader| {
            let manifest = reader.manifest(&parse(reference)?)?;
            let mut result = vec![];
            for (ordinal, member) in crate::array(&manifest, "records")?.iter().enumerate() {
                let envelope = reader.envelope(member)?;
                if codec
                    .as_ref()
                    .is_none_or(|codec| envelope["codec"] == *codec)
                {
                    result.push(json!({"ordinal":ordinal,"record":member,"envelope":envelope}));
                }
            }
            Ok(serde_json::to_string(&result)?)
        })
    }
    fn read<'py>(
        &self,
        py: Python<'py>,
        reference: &str,
    ) -> PyResult<(String, Bound<'py, PyBytes>)> {
        let record = self.with_reader(py, |r| r.read(&parse(reference)?))?;
        Ok((
            serde_json::to_string(&record.metadata).unwrap(),
            PyBytes::new(py, &record.payload),
        ))
    }
    #[pyo3(signature = (reference, start, stop, expected=None))]
    fn read_tensor_range<'py>(
        &self,
        py: Python<'py>,
        reference: &str,
        start: u64,
        stop: u64,
        expected: Option<&str>,
    ) -> PyResult<(String, Bound<'py, PyBytes>, u64)> {
        let (envelope, data, checked) = self.with_reader(py, |r| {
            r.read_tensor_range_checked(
                &parse(reference)?,
                start,
                stop,
                expected.map(parse).transpose()?.as_ref(),
            )
        })?;
        Ok((
            serde_json::to_string(&envelope).unwrap(),
            PyBytes::new(py, &data),
            checked,
        ))
    }
    #[pyo3(signature = (requests, workers=4, expected=None, read_ahead_bytes=1048576))]
    fn read_tensor_ranges<'py>(
        &self,
        py: Python<'py>,
        requests: &str,
        workers: usize,
        expected: Option<&str>,
        read_ahead_bytes: usize,
    ) -> PyResult<(String, Vec<Bound<'py, PyBytes>>, u64)> {
        let values = self.with_reader(py, |reader| {
            let requests: Vec<(Value, u64, u64)> = serde_json::from_str(requests)?;
            let expected: Vec<Value> = expected
                .map(serde_json::from_str)
                .transpose()?
                .unwrap_or_default();
            reader.read_tensor_ranges_checked(&requests, &expected, workers, read_ahead_bytes)
        })?;
        let checked = values.iter().map(|(_, _, checked)| checked).sum();
        let envelopes: Vec<_> = values.iter().map(|(envelope, _, _)| envelope).collect();
        let metadata = serde_json::to_string(&envelopes).unwrap();
        let payloads = values
            .iter()
            .map(|(_, data, _)| PyBytes::new(py, data))
            .collect();
        Ok((metadata, payloads, checked))
    }
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        self.check_process()?;
        py.allow_threads(|| {
            self.reader
                .lock()
                .map_err(|_| Error::new("RuntimeError", "Reader mutex poisoned"))?
                .take();
            Ok(())
        })
        .map_err(|e| exception(py, e))
    }
}
#[pyclass]
struct NativeStore {
    config: Config,
    writer: Mutex<Store>,
    pid: u32,
}
impl NativeStore {
    fn writer_access(&self) -> Result<MutexGuard<'_, Store>> {
        // Fault callbacks run while the writer is held. Waiting here would
        // deadlock a callback, or a caller that must let that callback return.
        let (token, poisoned) = match self.writer.try_lock() {
            Ok(guard) => (Some(guard), false),
            Err(TryLockError::WouldBlock) => (None, false),
            Err(TryLockError::Poisoned(error)) => (Some(error.into_inner()), true),
        };
        match crate::access::decide(token, poisoned) {
            crate::access::Access::Granted(guard) => Ok(guard),
            crate::access::Access::Busy => Err(Error::new(
                "ResourceLimitExceeded",
                "Writer has an in-flight operation",
            )),
            crate::access::Access::Poisoned => {
                Err(Error::new("RuntimeError", "Writer mutex poisoned"))
            }
        }
    }
    fn check_writer_process(&self) -> PyResult<()> {
        // A child must not acquire an inherited mutex or act on its parent's
        // unpublished file. The owning thread may not exist after fork.
        if !crate::incomplete::writer_process(self.pid, std::process::id()) {
            return Err(PyRuntimeError::new_err(
                "Writer cannot be reused after fork; create a fresh writer",
            ));
        }
        Ok(())
    }
}
#[pymethods]
impl NativeStore {
    #[new]
    fn new(py: Python<'_>, config_json: &str) -> PyResult<Self> {
        let config = config(config_json).map_err(|e| exception(py, e))?;
        Ok(Self {
            writer: Mutex::new(Store::new(config.clone())),
            config,
            pid: std::process::id(),
        })
    }
    fn read_session(&self) -> NativeReader {
        NativeReader {
            reader: Mutex::new(Some(Reader::new(self.config.clone()))),
            pid: std::process::id(),
        }
    }
    fn validation_scope(&self) -> NativeValidationScope {
        NativeValidationScope {
            state: Arc::new(ValidationScopeState {
                graph: Mutex::new(Some(GraphProof::new(&self.config))),
                closed: AtomicBool::new(false),
            }),
            pid: std::process::id(),
        }
    }
    fn write(
        &self,
        py: Python<'_>,
        metadata: &str,
        payloads: Vec<Py<PyAny>>,
        submission_id: &str,
        fault: Py<PyAny>,
    ) -> PyResult<String> {
        self.check_writer_process()?;
        let records = records(py, &self.config, metadata, payloads)?;
        run(py, &fault, |fault| {
            let mut store = self.writer_access()?;
            Ok(serde_json::to_string(&store.write_inputs(
                &records,
                submission_id,
                fault,
            )?)?)
        })
    }
    #[pyo3(signature = (groups, metadata, payloads, submission_id, fault, validation_scope=None))]
    #[allow(clippy::too_many_arguments)] // Preserve existing FFI arguments plus the optional scope.
    fn publish(
        &self,
        py: Python<'_>,
        groups: &str,
        metadata: &str,
        payloads: Vec<Py<PyAny>>,
        submission_id: &str,
        fault: Py<PyAny>,
        validation_scope: Option<PyRef<'_, NativeValidationScope>>,
    ) -> PyResult<String> {
        self.check_writer_process()?;
        let scope = validation_scope
            .as_ref()
            .map(|scope| scope.checked())
            .transpose()?;
        let result = (|| {
            let records = records(py, &self.config, metadata, payloads)?;
            let groups = parse(groups).map_err(|e| exception(py, e))?;
            run(py, &fault, |fault| {
                scoped(scope.as_ref(), |graph| {
                    let mut store = self.writer_access()?;
                    Ok(serde_json::to_string(&store.publish_inputs_with_graph(
                        &groups,
                        records,
                        submission_id,
                        fault,
                        graph,
                    )?)?)
                })
            })
        })();
        if result.is_err()
            && let Some(scope) = scope
        {
            scope.invalidate();
        }
        result
    }
    fn record_set(
        &self,
        py: Python<'_>,
        references: &str,
        dependencies: &str,
        submission_id: &str,
        fault: Py<PyAny>,
    ) -> PyResult<String> {
        self.check_writer_process()?;
        let refs: Vec<Value> =
            serde_json::from_str(references).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        let deps: Vec<Value> = serde_json::from_str(dependencies)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        run(py, &fault, |fault| {
            let mut store = self.writer_access()?;
            Ok(serde_json::to_string(&store.record_set(
                &refs,
                &deps,
                submission_id,
                fault,
            )?)?)
        })
    }
    fn inspect(&self, py: Python<'_>, reference: &str, verify: bool) -> PyResult<String> {
        py.allow_threads(|| {
            let r = parse(reference)?;
            Ok(serde_json::to_string(&self.config.inspect(&r, verify)?)?)
        })
        .map_err(|e| exception(py, e))
    }
    fn discover(&self, py: Python<'_>, partition: &str) -> PyResult<String> {
        py.allow_threads(|| Ok(serde_json::to_string(&self.config.discover(partition)?)?))
            .map_err(|e| exception(py, e))
    }
    fn manifest(&self, py: Python<'_>, reference: &str) -> PyResult<String> {
        py.allow_threads(|| {
            let r = parse(reference)?;
            Ok(serde_json::to_string(&self.config.manifest(&r)?)?)
        })
        .map_err(|e| exception(py, e))
    }
    #[pyo3(signature = (reference, task=None, attempt=None))]
    fn validate(
        &self,
        py: Python<'_>,
        reference: &str,
        task: Option<String>,
        attempt: Option<String>,
    ) -> PyResult<String> {
        py.allow_threads(|| {
            let r = parse(reference)?;
            Ok(serde_json::to_string(&self.config.validate(
                &r,
                task.as_deref(),
                attempt.as_deref(),
            )?)?)
        })
        .map_err(|e| exception(py, e))
    }
    fn read<'py>(
        &self,
        py: Python<'py>,
        reference: &str,
    ) -> PyResult<(String, Bound<'py, PyBytes>)> {
        let record = py
            .allow_threads(|| self.config.read(&parse(reference)?))
            .map_err(|e| exception(py, e))?;
        Ok((
            serde_json::to_string(&record.metadata).unwrap(),
            PyBytes::new(py, &record.payload),
        ))
    }
    fn read_range<'py>(
        &self,
        py: Python<'py>,
        reference: &str,
        start: u64,
        stop: u64,
        chunk: u64,
        checksums: Vec<String>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let data = py
            .allow_threads(|| {
                self.config
                    .read_range(&parse(reference)?, start, stop, chunk, &checksums)
            })
            .map_err(|e| exception(py, e))?;
        Ok(PyBytes::new(py, &data))
    }
    fn read_range_stats<'py>(
        &self,
        py: Python<'py>,
        reference: &str,
        start: u64,
        stop: u64,
        chunk: u64,
        checksums: Vec<String>,
    ) -> PyResult<(Bound<'py, PyBytes>, u64)> {
        let (data, checked) = py
            .allow_threads(|| {
                self.config
                    .read_range_counted(&parse(reference)?, start, stop, chunk, &checksums)
            })
            .map_err(|e| exception(py, e))?;
        Ok((PyBytes::new(py, &data), checked))
    }
    #[pyo3(signature = (reference, start, stop, expected=None))]
    fn read_tensor_range<'py>(
        &self,
        py: Python<'py>,
        reference: &str,
        start: u64,
        stop: u64,
        expected: Option<&str>,
    ) -> PyResult<(String, Bound<'py, PyBytes>, u64)> {
        let (envelope, data, checked) = py
            .allow_threads(|| {
                self.config.read_tensor_range_checked(
                    &parse(reference)?,
                    start,
                    stop,
                    expected.map(parse).transpose()?.as_ref(),
                )
            })
            .map_err(|e| exception(py, e))?;
        Ok((
            serde_json::to_string(&envelope).unwrap(),
            PyBytes::new(py, &data),
            checked,
        ))
    }
    fn metrics(&self, py: Python<'_>) -> PyResult<String> {
        self.check_writer_process()?;
        py.allow_threads(|| -> Result<String> {
            Ok(serde_json::to_string(&self.writer_access()?.metrics)?)
        })
        .map_err(|e| exception(py, e))
    }
    fn storage(
        &self,
        py: Python<'_>,
        method: &str,
        arguments: &str,
        fault: Py<PyAny>,
    ) -> PyResult<String> {
        self.check_writer_process()?;
        let args = parse(arguments).map_err(|e| exception(py, e))?;
        run(py, &fault, |fault| {
            if method == "collect" {
                return Ok(serde_json::to_string(&self.config.collect_garbage(fault)?)?);
            }
            let result = self.config.catalog(|c| match method {
                "retain" => {
                    c.retain(
                        &self.config,
                        crate::field(&args, "owner")?,
                        crate::array(&args, "refs")?,
                        fault,
                    )?;
                    Ok(Value::Null)
                }
                "release" => {
                    c.release(&self.config, &[crate::field(&args, "owner")?.into()], fault)?;
                    Ok(Value::Null)
                }
                "consume" => {
                    c.consume(&self.config, crate::array(&args, "refs")?, fault)?;
                    Ok(Value::Null)
                }
                _ => crate::fail("ValueError", "Unknown storage operation"),
            })?;
            Ok(serde_json::to_string(&result)?)
        })
    }
    fn seal(&self, py: Python<'_>, fault: Py<PyAny>) -> PyResult<()> {
        self.check_writer_process()?;
        run(py, &fault, |fault| self.writer_access()?.seal(fault))
    }
    fn writer_id(&self, py: Python<'_>) -> PyResult<String> {
        self.check_writer_process()?;
        py.allow_threads(|| Ok(self.writer_access()?.writer_id().to_owned()))
            .map_err(|e| exception(py, e))
    }
    fn retire_writer(
        &self,
        py: Python<'_>,
        writer_id: &str,
        policy: &str,
        fault: Py<PyAny>,
    ) -> PyResult<String> {
        self.check_writer_process()?;
        let policy = match policy {
            "recover" => crate::gc::RetirementPolicy::Recover,
            "abandon" => crate::gc::RetirementPolicy::Abandon,
            _ => {
                return Err(exception(
                    py,
                    Error::new(
                        "ValueError",
                        "Expected recover or abandon retirement policy",
                    ),
                ));
            }
        };
        run(py, &fault, |fault| {
            Ok(serde_json::to_string(
                &self.config.retire_writer(writer_id, policy, fault)?,
            )?)
        })
    }
    fn publication_receipt(
        &self,
        py: Python<'_>,
        writer_id: &str,
        api: &str,
        submission_id: &str,
        fault: Py<PyAny>,
    ) -> PyResult<String> {
        self.check_writer_process()?;
        run(py, &fault, |fault| {
            Ok(serde_json::to_string(&self.config.publication_receipt(
                writer_id,
                api,
                submission_id,
                fault,
            )?)?)
        })
    }
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        self.check_writer_process()?;
        py.allow_threads(|| self.writer_access()?.close())
            .map_err(|e| exception(py, e))
    }
}
#[pyclass]
struct NativeCoordinator {
    owner: Mutex<Coordinator>,
}
#[pymethods]
impl NativeCoordinator {
    #[new]
    fn new(py: Python<'_>, config_json: &str, options: &str, fault: Py<PyAny>) -> PyResult<Self> {
        let config = config(config_json).map_err(|e| exception(py, e))?;
        let options = parse(options).map_err(|e| exception(py, e))?;
        let owner = run(py, &fault, |f| Coordinator::open(config, &options, f))?;
        Ok(Self {
            owner: Mutex::new(owner),
        })
    }
    #[pyo3(signature = (method, args, now, fault, validation_scope=None))]
    fn call(
        &self,
        py: Python<'_>,
        method: &str,
        args: &str,
        now: f64,
        fault: Py<PyAny>,
        validation_scope: Option<PyRef<'_, NativeValidationScope>>,
    ) -> PyResult<String> {
        let scope = validation_scope
            .as_ref()
            .map(|scope| scope.checked())
            .transpose()?;
        let result = (|| {
            let args = parse(args).map_err(|e| exception(py, e))?;
            run(py, &fault, |f| {
                scoped(scope.as_ref(), |graph| {
                    let result = self
                        .owner
                        .lock()
                        .unwrap()
                        .call_with_graph(method, &args, now, f, graph)?;
                    Ok(serde_json::to_string(&result)?)
                })
            })
        })();
        if result.is_err()
            && let Some(scope) = scope
        {
            scope.invalidate();
        }
        result
    }
    fn diagnostics(&self, py: Python<'_>) -> String {
        py.allow_threads(||{let q=self.owner.lock().unwrap();json!({"path":q.journal.path,"sequence":q.journal.sequence,"poisoned":q.journal.poisoned,"closed":q.journal.closed(),"recovery_seconds":q.recovery_seconds}).to_string()})
    }
    fn close(&self, py: Python<'_>) {
        py.allow_threads(|| self.owner.lock().unwrap().close());
    }
}
#[pyclass]
struct NativeJournal {
    journal: Mutex<Journal>,
}
#[pymethods]
impl NativeJournal {
    #[new]
    fn new(
        py: Python<'_>,
        root: &str,
        identity: &str,
        recover: bool,
        read_only: bool,
        fault: Py<PyAny>,
    ) -> PyResult<Self> {
        let identity = parse(identity).map_err(|e| exception(py, e))?;
        let journal = run(py, &fault, |f| {
            Journal::open(std::path::Path::new(root), &identity, recover, read_only, f)
        })?;
        Ok(Self {
            journal: Mutex::new(journal),
        })
    }
    fn info(&self, py: Python<'_>) -> String {
        py.allow_threads(||{let j=self.journal.lock().unwrap();json!({"sequence":j.sequence,"transactions":j.transactions,"poisoned":j.poisoned,"closed":j.closed(),"incomplete_tail_bytes":j.incomplete_tail}).to_string()})
    }
    fn append(&self, py: Python<'_>, events: &str, fault: Py<PyAny>) -> PyResult<u64> {
        let events = parse(events).map_err(|e| exception(py, e))?;
        run(py, &fault, |f| {
            self.journal.lock().unwrap().append(&events, f)
        })
    }
    fn close(&self, py: Python<'_>) {
        py.allow_threads(|| self.journal.lock().unwrap().close());
    }
}
#[pyfunction]
fn filesystem_path(py: Python<'_>, root: &str, relative: &str) -> PyResult<String> {
    py.allow_threads(|| {
        Ok(crate::store::storage_path(Path::new(root), relative)?
            .to_string_lossy()
            .into_owned())
    })
    .map_err(|e| exception(py, e))
}
#[pyfunction]
fn read_committed<'py>(
    py: Python<'py>,
    root: &str,
    relative: &str,
) -> PyResult<Bound<'py, PyBytes>> {
    let data = py
        .allow_threads(|| -> Result<Vec<u8>> {
            let mut data = vec![];
            crate::store::open_committed(Path::new(root), relative)?.read_to_end(&mut data)?;
            Ok(data)
        })
        .map_err(|e| exception(py, e))?;
    Ok(PyBytes::new(py, &data))
}
#[pyfunction]
fn sync_directory(py: Python<'_>, path: &str, fault: Py<PyAny>) -> PyResult<()> {
    run(py, &fault, |f| crate::store::sync_dir(Path::new(path), f))
}
#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("MAX_RECORDS", crate::store::MAX_RECORDS)?;
    m.add_class::<NativeStore>()?;
    m.add_class::<NativeReader>()?;
    m.add_class::<NativeCoordinator>()?;
    m.add_class::<NativeValidationScope>()?;
    m.add_class::<NativeJournal>()?;
    m.add_function(wrap_pyfunction!(filesystem_path, m)?)?;
    m.add_function(wrap_pyfunction!(read_committed, m)?)?;
    m.add_function(wrap_pyfunction!(sync_directory, m)?)?;
    Ok(())
}
