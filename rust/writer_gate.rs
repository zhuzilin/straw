//! Fixed-inode writer gates and durable per-incarnation operation evidence.
//!
//! Every writer mutation holds a fresh exclusive gate descriptor. A retiree
//! keeps the same gate locked from fencing through its final receipt. Gates and
//! fence markers are permanent: deleting/replacing them would permit ABA reuse.
use crate::journal::Journal;
use crate::store::{mkdir, sync, sync_dir};
use crate::{Error, Fault, Result, bytes, fail, field};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

fn identity(file: &File) -> Result<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata()?;
        Ok((m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        fail(
            "UnsafeRecovery",
            "Writer gates require POSIX file identities",
        )
    }
}
fn descriptor(run: &str, writer: &str) -> Value {
    json!({"version":1,"run_id":run,"writer_id":writer})
}
pub(crate) fn operation_key(writer: &str, api: crate::finalization::Api, request: &str) -> String {
    match api {
        crate::finalization::Api::Records => format!("records:{request}"),
        crate::finalization::Api::RecordSet => format!("publication:{writer}:record_set:{request}"),
        crate::finalization::Api::Publish => format!("publication:{writer}:publish:{request}"),
    }
}
pub(crate) fn intermediate_handoff(
    run: &str,
    writer: &str,
    key: &str,
    refs: &[Value],
) -> Result<String> {
    crate::digest(
        &json!({"kind":"writer-intermediate","run_id":run,"writer_id":writer,"operation_key":key,"intermediates":refs}),
    )
}
fn header(file: &mut File, expected: &Value) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let expected_bytes = bytes(expected)?;
    let mut raw = Vec::new();
    file.take(expected_bytes.len() as u64 + 1)
        .read_to_end(&mut raw)?;
    if raw != expected_bytes {
        return fail(
            "UnsafeRecovery",
            "Writer gate identity is missing, incomplete or mismatched",
        );
    }
    Ok(())
}
fn control(root: &Path, writer: &str) -> Result<PathBuf> {
    if !crate::writer_retirement::valid_writer_name(writer.as_bytes()) {
        return fail("ValueError", "Invalid writer incarnation");
    }
    Ok(root
        .join("control")
        .join("writers")
        .join(&writer[..2])
        .join(writer))
}

struct NativeGate {
    directory: PathBuf,
}
impl crate::writer_gate_flow::Model for NativeGate {
    type File = File;
}
impl crate::writer_gate_flow::Primitives for NativeGate {
    type Error = Error;
    fn open(&mut self) -> Result<File> {
        OpenOptions::new().read(true).write(true).open(self.directory.join("gate"))
            .map_err(|error| if error.kind() == std::io::ErrorKind::NotFound {
                Error::new("UnsafeRecovery", "No registered writer gate; legacy or uninitialized writers require separate fencing")
            } else { error.into() })
    }
    fn lock(&mut self, file: &File, nonblocking: bool) -> Result<()> {
        if !nonblocking {
            file.lock()?;
            return Ok(());
        }
        match file.try_lock() {
            Ok(()) => Ok(()),
            Err(std::fs::TryLockError::WouldBlock) => fail(
                "WriterBusy",
                "Writer still holds its mutation gate; retry after it finishes or exits",
            ),
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }
    fn identity(&mut self, file: &File) -> Result<(u64, u64)> {
        identity(file)
    }
    fn read_header(&mut self, file: &mut File, limit: usize) -> Result<Vec<u8>> {
        file.seek(SeekFrom::Start(0))?;
        let mut raw = Vec::new();
        file.take(limit as u64).read_to_end(&mut raw)?;
        Ok(raw)
    }
    fn fenced(&mut self) -> Result<bool> {
        Ok(self.directory.join("fenced").try_exists()?)
    }
    fn named_identity(&mut self) -> Result<(u64, u64)> {
        identity(&File::open(self.directory.join("gate"))?)
    }
}
fn access_error(error: crate::writer_gate_flow::Failure<Error>) -> Error {
    use crate::writer_gate_flow::Failure;
    match error {
        Failure::Io(error) => error,
        Failure::Context => Error::new("UnsafeRecovery", "Writer gate storage context changed"),
        Failure::Identity => Error::new("UnsafeRecovery", "Writer gate inode changed"),
        Failure::Header => Error::new(
            "UnsafeRecovery",
            "Writer gate identity is missing, incomplete or mismatched",
        ),
        Failure::Fenced => Error::new(
            "WriterFenced",
            "Writer incarnation is permanently fenced; create a new writer",
        ),
    }
}

pub(crate) struct Gate {
    root: PathBuf,
    run: String,
    writer: String,
    directory: PathBuf,
    expected: (u64, u64),
}
/// Owning this descriptor is the native critical-section lifetime. It cannot be
/// cloned or exposed to callers, and normal/error/unwind exits close it.
pub(crate) struct Permit {
    access: crate::writer_gate_flow::Access<NativeGate>,
    directory: PathBuf,
    descriptor: Value,
    writer_directory: PathBuf,
}
impl Gate {
    pub(crate) fn create(
        root: &Path,
        run: &str,
        writer: &str,
        fault: &mut Fault<'_>,
    ) -> Result<Self> {
        let mut scoped = |point: &str| fault(&format!("writer_gate:{point}"));
        let fault: &mut Fault<'_> = &mut scoped;
        let directory = control(root, writer)?;
        mkdir(&directory, fault)?;
        let path = directory.join("gate");
        let expected_header = descriptor(run, writer);
        let mut file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.lock()?;
                file.write_all(&bytes(&expected_header)?)?;
                sync(&file, fault)?;
                sync_dir(&directory, fault)?;
                file
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let file = OpenOptions::new().read(true).write(true).open(&path)?;
                file.lock()?;
                file
            }
            Err(error) => return Err(error.into()),
        };
        header(&mut file, &expected_header)?;
        let expected = identity(&file)?;
        let registry_root = directory.join("operations");
        let recover = registry_root.join("control/journal.log").try_exists()?;
        let mut journal = Journal::open(&registry_root, &expected_header, recover, false, fault)?;
        journal.close();
        file.unlock()?;
        Ok(Self {
            root: root.canonicalize()?,
            run: run.into(),
            writer: writer.into(),
            directory,
            expected,
        })
    }
    pub(crate) fn enter(&self, root: &Path, run: &str) -> Result<Permit> {
        let expected_header = descriptor(&self.run, &self.writer);
        let access = crate::writer_gate_flow::acquire(
            NativeGate {
                directory: self.directory.clone(),
            },
            Some(self.expected),
            &bytes(&expected_header)?,
            root.canonicalize()? == self.root && run == self.run,
            false,
            false,
        )
        .map_err(access_error)?;
        Ok(Permit {
            access,
            directory: self.directory.clone(),
            descriptor: expected_header,
            writer_directory: self
                .root
                .join("raw")
                .join(&self.writer[..2])
                .join(&self.writer),
        })
    }
}
impl Permit {
    pub(crate) fn retire(root: &Path, run: &str, writer: &str) -> Result<Self> {
        let directory = control(root, writer)?;
        let expected_header = descriptor(run, writer);
        let access = crate::writer_gate_flow::acquire(
            NativeGate {
                directory: directory.clone(),
            },
            None,
            &bytes(&expected_header)?,
            true,
            true,
            true,
        )
        .map_err(access_error)?;
        Ok(Self {
            access,
            directory,
            descriptor: expected_header,
            writer_directory: root.join("raw").join(&writer[..2]).join(writer),
        })
    }
    fn bound(&mut self) -> Result<()> {
        self.access.bound().map_err(access_error)
    }
    pub(crate) fn registry(&mut self, fault: &mut Fault<'_>) -> Result<Registry> {
        self.bound()?;
        Registry::open(
            &self.directory.join("operations"),
            &self.descriptor,
            false,
            fault,
        )
    }
    pub(crate) fn receipt(
        &mut self,
        key: &str,
        fault: &mut Fault<'_>,
    ) -> Result<Option<Vec<Value>>> {
        self.bound()?;
        let root = self.directory.join("operations");
        // A complete frame left by a failed sync may be visible without being
        // durable yet. Confirm that prefix without truncating or appending it.
        sync(&File::open(root.join("control/journal.log"))?, fault)?;
        let registry = Registry::open(&root, &self.descriptor, true, fault)?;
        Ok(crate::writer_registry::receipt(&registry.operations, key).map(|roots| roots.to_vec()))
    }
    pub(crate) fn writer_directory(&self) -> &Path {
        &self.writer_directory
    }
    pub(crate) fn durable_marker(
        &mut self,
        marker: crate::writer_gate_flow::Marker,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.bound()?;
        let mut io = MarkerIo {
            permit: self,
            fault,
        };
        crate::writer_gate_flow::persist_marker(&mut io, marker)
    }
    pub(crate) fn drain_files(&mut self, fault: &mut Fault<'_>) -> Result<()> {
        self.bound()?;
        let mut io = CleanupIo {
            permit: self,
            fault,
        };
        crate::writer_gate_flow::drain(&mut io)
    }
}

struct CleanupIo<'a, 'f> {
    permit: &'a mut Permit,
    fault: &'a mut Fault<'f>,
}
impl crate::writer_gate_flow::CleanupModel for CleanupIo<'_, '_> {
    type Path = PathBuf;
    type IoError = std::io::Error;
}
impl crate::writer_gate_flow::Cleanup for CleanupIo<'_, '_> {
    type Error = Error;
    fn prepare(&mut self) -> Result<()> {
        mkdir(self.permit.writer_directory(), self.fault)
    }
    fn list(&mut self) -> Result<Vec<crate::writer_gate_flow::Entry<PathBuf>>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(self.permit.writer_directory())? {
            let path = entry?.path();
            let kind = crate::writer_gate_flow::payload_kind(
                path.extension()
                    .and_then(|part| part.to_str())
                    .unwrap_or(""),
            );
            entries.push(crate::writer_gate_flow::Entry { path, kind });
        }
        Ok(entries)
    }
    fn sync_file(&mut self, path: &PathBuf) -> Result<()> {
        sync(&File::open(path)?, self.fault)
    }
    fn unlink(&mut self, path: &PathBuf) -> std::io::Result<()> {
        fs::remove_file(path)
    }
    fn is_missing(&self, error: &std::io::Error) -> bool {
        error.kind() == std::io::ErrorKind::NotFound
    }
    fn io_error(&self, error: std::io::Error) -> Error {
        error.into()
    }
    fn checkpoint(&mut self, before: bool) -> Result<()> {
        (self.fault)(if before {
            "before_retirement_unlink_partial"
        } else {
            "after_retirement_unlink_partial"
        })
    }
    fn sync_directory(&mut self) -> Result<()> {
        sync_dir(self.permit.writer_directory(), self.fault)
    }
}

struct MarkerIo<'a, 'f> {
    permit: &'a mut Permit,
    fault: &'a mut Fault<'f>,
}
impl crate::writer_gate_flow::PersistenceModel for MarkerIo<'_, '_> {
    type File = File;
    type IoError = std::io::Error;
}
impl crate::writer_gate_flow::Persistence for MarkerIo<'_, '_> {
    type Error = Error;
    fn create(&mut self, marker: crate::writer_gate_flow::Marker) -> std::io::Result<File> {
        OpenOptions::new().write(true).create_new(true).open(
            self.permit
                .directory
                .join(crate::writer_gate_flow::marker_name(marker)),
        )
    }
    fn open(&mut self, marker: crate::writer_gate_flow::Marker) -> std::io::Result<File> {
        File::open(
            self.permit
                .directory
                .join(crate::writer_gate_flow::marker_name(marker)),
        )
    }
    fn already_exists(&self, error: &std::io::Error) -> bool {
        error.kind() == std::io::ErrorKind::AlreadyExists
    }
    fn io_error(&self, error: std::io::Error) -> Error {
        error.into()
    }
    fn sync_file(&mut self, file: &File, _marker: crate::writer_gate_flow::Marker) -> Result<()> {
        sync(file, self.fault)
    }
    fn sync_directory(&mut self, _marker: crate::writer_gate_flow::Marker) -> Result<()> {
        sync_dir(&self.permit.directory, self.fault)
    }
}

pub(crate) type Operation = crate::writer_registry::Operation;
pub(crate) struct Registry {
    journal: Journal,
    operations: Vec<Operation>,
    retired_results: Vec<Value>,
}
impl Registry {
    fn open(root: &Path, identity: &Value, read_only: bool, fault: &mut Fault<'_>) -> Result<Self> {
        let journal = Journal::open(root, identity, true, read_only, &mut |point| {
            fault(&format!("writer_registry:{point}"))
        })?;
        let mut operations = Vec::new();
        let mut retired_results = Vec::new();
        if !crate::writer_registry::replay(
            &mut operations,
            &mut retired_results,
            &journal.transactions,
        ) {
            return fail("CorruptData", "Invalid writer operation history");
        }
        let prefix = format!("publication:{}:", field(identity, "writer_id")?);
        for op in &operations {
            if let Some(owner) = &op.evidence.owner
                && (!owner.starts_with(&prefix) || &op.evidence.key != owner)
            {
                return fail(
                    "CorruptData",
                    "Writer operation owner belongs to another incarnation",
                );
            }
        }
        Ok(Self {
            journal,
            operations,
            retired_results,
        })
    }
    fn apply_to(
        operations: &mut Vec<Operation>,
        retired_results: &mut Vec<Value>,
        sequence: u64,
        value: &Value,
    ) -> Result<()> {
        let event = crate::writer_registry::decode(value)
            .ok_or_else(|| Error::new("CorruptData", "Invalid writer registry event schema"))?;
        if !crate::writer_registry::apply_event(operations, retired_results, sequence, &event) {
            return fail("CorruptData", "Invalid writer operation transition");
        }
        Ok(())
    }
    fn append(&mut self, event: Value, fault: &mut Fault<'_>) -> Result<()> {
        let sequence = self.journal.append(&json!([event]), &mut |point| {
            fault(&format!("writer_registry:{point}"))
        })?;
        Self::apply_to(
            &mut self.operations,
            &mut self.retired_results,
            sequence,
            &event,
        )
    }
    pub(crate) fn begin(
        &mut self,
        key: &str,
        owner: Option<&str>,
        dependencies: &[Value],
        fault: &mut Fault<'_>,
    ) -> Result<u64> {
        let sequence = self.journal.sequence;
        self.append(json!({"type":"Begin","sequence":sequence,"key":key,"owner":owner,"dependencies":dependencies}),fault)?;
        Ok(sequence)
    }
    pub(crate) fn results(
        &mut self,
        operation: u64,
        results: &[Value],
        intermediates: &[Value],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.append(
            json!({"type":"Results","operation":operation,"results":results,"intermediates":intermediates}),
            fault,
        )
    }
    pub(crate) fn retired(
        &mut self,
        operations: &[u64],
        results: &[Value],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.append(
            json!({"type":"Retired","operations":operations,"results":results}),
            fault,
        )
    }
    pub(crate) fn retired_results(&self) -> Vec<Value> {
        self.retired_results.clone()
    }
    pub(crate) fn done_owner(&mut self, owner: &str, fault: &mut Fault<'_>) -> Result<()> {
        self.append(json!({"type":"DoneOwner","owner":owner}), fault)
    }
    pub(crate) fn done_key(&mut self, key: &str, fault: &mut Fault<'_>) -> Result<()> {
        self.append(json!({"type":"DoneKey","key":key}), fault)
    }
    pub(crate) fn pending(&self) -> Vec<Operation> {
        self.operations
            .iter()
            .filter(|op| !op.done)
            .cloned()
            .collect()
    }
}
