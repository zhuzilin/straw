//! Cross-queue ownership catalog. One append log/lock inode per storage pool.
//!
//! Writers register open packs before creating them. Publications and queue
//! owners retain transitive pack paths before releasing their predecessor.
//! Unknown/crashed writers and uncertain publications leak safely; no TTLs.
use crate::catalog_event::valid_handoff_id;
#[cfg(test)]
use crate::field;
#[cfg(test)]
use crate::framing;
use crate::framing::Kind;
use crate::protocol::{self, PackStatus};
use crate::store::{Config, Reader};
use crate::{Error, Fault, Result, array, digest, fail};
use serde_json::{Value, json};
#[cfg(test)]
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::File;
#[cfg(test)]
use std::io::Write;
use std::path::{Path, PathBuf};

fn catalog_append_error(error: crate::catalog_append::Error<Error, Error>) -> Error {
    match error {
        crate::catalog_append::Error::Decode(error) => catalog_decode_error(error),
        crate::catalog_append::Error::Json(error) => error.into(),
        crate::catalog_append::Error::Append(crate::catalog_log::AppendFailure::State(error)) => {
            log_error(error)
        }
        crate::catalog_append::Error::Append(crate::catalog_log::AppendFailure::Write(error)) => {
            crate::wal_file::append_error(error)
        }
    }
}

fn collection_error(
    error: crate::gc_batch::Failure<crate::gc_collection::Error<Error, Error, Error>>,
) -> Error {
    match error {
        crate::gc_batch::Failure::Effect(error) => match error {
            crate::gc_collection::Error::Physical(error)
            | crate::gc_collection::Error::Point(error) => error,
            crate::gc_collection::Error::Receipt(error)
            | crate::gc_collection::Error::Mark(error) => catalog_append_error(error),
            crate::gc_collection::Error::Context => Error::new(
                "UnsafeRecovery",
                "Catalog requires a fresh locked recovery transaction",
            ),
            crate::gc_collection::Error::Directory => {
                Error::new("UnsafeRecovery", "Directory barriers did not finish")
            }
        },
        crate::gc_batch::Failure::Protocol => {
            Error::new("UnsafeRecovery", "Deletion batch did not finish")
        }
        crate::gc_batch::Failure::Accounting => {
            Error::new("CorruptData", "Deletion accounting exceeds u64")
        }
    }
}

fn queue_collection_error(error: crate::gc_collection::QueueFailure<Error, Error, Error>) -> Error {
    match error {
        crate::gc_collection::QueueFailure::Collect(error) => collection_error(error),
        crate::gc_collection::QueueFailure::Prune(error) => catalog_append_error(error),
        crate::gc_collection::QueueFailure::Ownership(error) => match error {
            crate::queue_gc::OwnershipError::Missing => Error::new(
                "UnsafeRecovery",
                "Live queue root has no durable storage owner",
            ),
            crate::queue_gc::OwnershipError::Invalid => Error::new(
                "UnsafeRecovery",
                "Live queue root has invalid storage ownership",
            ),
        },
        crate::gc_collection::QueueFailure::Reserved => Error::new(
            "ValueError",
            "publication: is reserved for durable writer operations",
        ),
    }
}

fn log_error(error: crate::catalog_log::Failure) -> Error {
    use crate::catalog_log::Failure;
    match error {
        Failure::InvalidOperation(_) => {
            Error::new("CorruptData", "Reclamation receipt without tombstone")
        }
        Failure::Boundary => Error::new("CorruptData", "Catalog frame or sequence outside bounds"),
        Failure::Truncated => Error::new(
            "UnsafeRecovery",
            "Storage catalog durable prefix was truncated",
        ),
        Failure::Unavailable => Error::new("UnsafeRecovery", "Catalog requires fresh recovery"),
        Failure::Mismatch => Error::new(
            "UnsafeRecovery",
            "Catalog replay boundary does not match applied records",
        ),
    }
}

/// Successful authentication of immutable roots, bound to their storage pool.
/// This is not a pin or an ownership grant; commit must recheck current owners.
pub struct VerifiedReactivation {
    root: PathBuf,
    run_id: String,
    roots: Vec<(String, Value)>,
    verified: HashSet<String>,
}

impl VerifiedReactivation {
    fn verify(
        config: &Config,
        roots: &[(String, Value)],
        missing: Vec<Value>,
        fault: &mut Fault<'_>,
    ) -> Result<Self> {
        let mut verified = HashSet::new();
        if !missing.is_empty() {
            fault("before_reactivation_verify")?;
            let mut reader = Reader::new(config.clone());
            for reference in missing {
                reader.validate(&reference, None, None)?;
                let mut pending = vec![reference.clone()];
                let mut checked = HashSet::new();
                while let Some(reference) = pending.pop() {
                    if !checked.insert(digest(&reference)?) {
                        continue;
                    }
                    let manifest = reader.manifest(&reference)?;
                    for record in array(&manifest, "records")? {
                        reader.verify(record)?;
                    }
                    pending.extend(array(&manifest, "dependencies")?.iter().cloned());
                }
                verified.insert(digest(&reference)?);
            }
            fault("after_reactivation_verify")?;
        }
        Ok(Self {
            root: config.root.clone(),
            run_id: config.run_id.clone(),
            roots: roots.to_vec(),
            verified,
        })
    }
}

impl Config {
    pub(crate) fn verify_queue_reactivation(
        &self,
        prefix: &str,
        roots: &[(String, Value)],
        fault: &mut Fault<'_>,
    ) -> Result<VerifiedReactivation> {
        writer_retirement::queue_owners(prefix, roots.iter().map(|(owner, _)| owner.as_str()))?;
        let missing = if roots.is_empty() {
            Vec::new()
        } else {
            self.catalog(|c| c.queue_reactivation_candidates(self, prefix, roots))?
                .unwrap_or_default()
        };
        VerifiedReactivation::verify(self, roots, missing, fault)
    }

    /// Snapshot candidates under the catalog lock, then authenticate payloads
    /// with no catalog lock/mutex held. GC may race and make this attempt fail.
    pub fn verify_reactivation(
        &self,
        roots: &[(String, Value)],
        fault: &mut Fault<'_>,
    ) -> Result<VerifiedReactivation> {
        let missing = if roots.is_empty() {
            Vec::new()
        } else {
            self.catalog(|c| c.reactivation_candidates(self, roots))?
                .unwrap_or_default()
        };
        VerifiedReactivation::verify(self, roots, missing, fault)
    }

    /// Reactivate after unlocked authentication and a fresh locked decision.
    pub fn retain_reactivated(
        &self,
        roots: &[(String, Value)],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        let verified = self.verify_reactivation(roots, fault)?;
        self.catalog(|c| c.retain_verified(self, &verified, fault))?;
        Ok(())
    }
}

fn validate_staging_edges(
    known: &HashMap<String, Vec<String>>,
    additions: &HashMap<String, Vec<String>>,
) -> Result<()> {
    if !crate::catalog_event::valid_staging_edges(known, additions) {
        return fail(
            "CorruptData",
            "Incomplete or conflicting staging dependency graph",
        );
    }
    Ok(())
}

// Per-node reading remains separate from the graph traversal and its cache.
struct ReferenceDigest;

impl crate::closure_node::Digest for ReferenceDigest {
    type Error = Error;

    fn digest(&mut self, value: &Value) -> Result<String> {
        digest(value)
    }
}

struct StagingReader<'a, 'f> {
    reader: Reader,
    fault: &'a mut Fault<'f>,
}
impl crate::closure::SourceModel for StagingReader<'_, '_> {
    type Reference = Value;
}
impl crate::staging_consumption::Source for StagingReader<'_, '_> {
    type Error = Error;
    fn manifest(&mut self, reference: &Value) -> Result<Value> {
        self.reader.manifest(reference)
    }
    fn point(&mut self, name: &str) -> Result<()> {
        (self.fault)(name)
    }
}
fn staging_error(error: crate::staging_consumption::Error<Error, Error>) -> Error {
    use crate::staging_consumption::Error as Failure;
    match error {
        Failure::Read(error) | Failure::Digest(error) => error,
        Failure::Reference => Error::new(
            "InvalidReference",
            "Expected a record or record-set reference",
        ),
        Failure::Missing => Error::new("CorruptData", "Missing persisted staging dependency node"),
        Failure::Dependencies => Error::new("InvalidReference", "Missing array dependencies"),
        Failure::Records => Error::new("InvalidReference", "Missing array records"),
    }
}

fn closure_node_error(error: crate::closure_node::Failure<Error>) -> Error {
    use crate::closure_node::Failure;
    let message = match error {
        Failure::Digest(error) => return error,
        Failure::Segment | Failure::Path => "Missing string path",
        Failure::Records => "Missing array records",
        Failure::Dependencies => "Missing array dependencies",
    };
    Error::new("InvalidReference", message)
}

struct ClosureReader<'a> {
    namespace: &'a mut crate::store::NamespaceState,
    inspected: &'a mut HashSet<String>,
    reader: &'a mut Reader,
    deleted: &'a HashSet<String>,
}

impl crate::closure::SourceModel for ClosureReader<'_> {
    type Reference = Value;
}

impl crate::closure::Source for ClosureReader<'_> {
    type Error = Error;

    fn expand(
        &mut self,
        _key: &String,
        reference: &Value,
    ) -> Result<crate::closure::Expansion<Value>> {
        use crate::closure_node::{Admission, Kind, children, kind, segment};
        let pack = crate::closure_node::live_pack(self.namespace, reference, self.deleted)
            .map_err(|error| match error {
                Admission::Field(error) => closure_node_error(error),
                Admission::Namespace(error) => error,
                Admission::Reference => Error::new(
                    "InvalidReference",
                    "Expected a record or record-set reference",
                ),
                Admission::Reclaimed => Error::new(
                    "InvalidReference",
                    "Reference has been reclaimed; retain before releasing its source owner",
                ),
            })?;
        let children = match kind(reference) {
            Kind::Manifest => {
                let contents = self.reader.manifest(reference)?;
                children(&mut ReferenceDigest, &contents).map_err(closure_node_error)?
            }
            Kind::Record => {
                let segment = segment(reference, false).map_err(closure_node_error)?;
                let key = digest(segment)?;
                if self.inspected.insert(key) {
                    self.reader.index(segment)?;
                }
                Vec::new()
            }
            Kind::Invalid => {
                return fail(
                    "InvalidReference",
                    "Expected a record or record-set reference",
                );
            }
        };
        Ok(crate::closure::Expansion { pack, children })
    }
}

fn catalog_decode_error(error: crate::catalog_decode::Failure<Error>) -> Error {
    use crate::catalog_decode::Failure;
    use crate::catalog_json::DecodeError;
    let message = match error {
        Failure::Path(error) => return error,
        Failure::Receipts => "Invalid staging handoff receipts",
        Failure::Graph => "Incomplete or conflicting staging dependency graph",
        Failure::Unsupported => "Unsupported catalog path encoding",
        Failure::Wire(error) => match error {
            DecodeError::Object => "Invalid catalog event",
            DecodeError::Encoding => "Invalid catalog path encoding",
            DecodeError::Owners => "Invalid catalog owners",
            DecodeError::Packs => "Invalid pack state",
            DecodeError::Deleted => "Invalid tombstone",
            DecodeError::Reclaimed => "Invalid reclamation receipt",
            DecodeError::Edges => "Invalid staging dependency graph",
            DecodeError::Receipts => "Invalid staging handoff receipts",
        },
    };
    Error::new("CorruptData", message)
}

fn catalog_replay_error(
    error: crate::wal_recovery::Failure<
        Error,
        crate::catalog_target::Error<crate::catalog_frame::Error<Error>>,
    >,
) -> Error {
    use crate::wal_recovery::Failure;
    let mapped = match error {
        Failure::Device(error) => Failure::Device(error),
        Failure::Malformed(reason) => Failure::Malformed(reason),
        Failure::Apply(crate::catalog_target::Error::Decode(error)) => {
            Failure::Apply(match error {
                crate::catalog_frame::Error::Json(error) => error.into(),
                crate::catalog_frame::Error::Run => {
                    Error::new("UnsafeRecovery", "Storage pool run identity mismatch")
                }
                crate::catalog_frame::Error::Event(error) => catalog_decode_error(error),
            })
        }
        Failure::Apply(crate::catalog_target::Error::State(error)) => {
            Failure::Apply(log_error(error))
        }
    };
    crate::wal_file::error(mapped)
}

pub type Catalog = crate::catalog_runtime::Catalog<crate::store::NamespaceSource>;

/// Borrowed access to a recovered catalog while its transaction owns the lock.
///
/// Operations keep their usual names, so inferred callback types need no change:
/// ```no_run
/// # use straw::{Result, gc::Catalog, store::Config};
/// # fn example(config: &Config) -> Result<()> {
/// let mut catalog = Catalog::open(config)?;
/// catalog.transaction(config, |locked| {
///     locked.pack(config, "raw/example", true, &mut |_| Ok(()))
/// })?;
/// # Ok(()) }
/// ```
/// The owning catalog cannot be moved out of the callback:
/// ```compile_fail,E0308
/// # use straw::{Result, gc::Catalog, store::Config};
/// # fn example(config: &Config) -> Result<()> {
/// let mut catalog = Catalog::open(config)?;
/// let mut escaped = None;
/// catalog.transaction(config, |locked| {
///     escaped = Some(std::mem::replace(locked, Catalog::open(config)?));
///     Ok(())
/// })?;
/// # Ok(()) }
/// ```
/// Read-only dereferencing does not provide a mutable catalog escape:
/// ```compile_fail,E0596
/// # use straw::{Result, gc::Catalog, store::Config};
/// # fn example(config: &Config) -> Result<()> {
/// let mut catalog = Catalog::open(config)?;
/// catalog.transaction(config, |locked| {
///     let _escaped = std::mem::replace(&mut **locked, Catalog::open(config)?);
///     Ok(())
/// })?;
/// # Ok(()) }
/// ```
/// Borrowed transaction access cannot outlive the callback either:
/// ```compile_fail,E0521
/// # use straw::{Result, gc::Catalog, store::Config};
/// # fn example(config: &Config) -> Result<()> {
/// let mut catalog = Catalog::open(config)?;
/// let mut escaped = None;
/// catalog.transaction(config, |locked| {
///     escaped = Some(locked);
///     Ok(())
/// })?;
/// # Ok(()) }
/// ```
pub struct LockedCatalog<'a> {
    catalog: &'a mut Catalog,
}

impl std::ops::Deref for LockedCatalog<'_> {
    type Target = Catalog;

    fn deref(&self) -> &Catalog {
        self.catalog
    }
}

impl LockedCatalog<'_> {
    pub fn pack(
        &mut self,
        config: &Config,
        path: &str,
        sealed: bool,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog.pack(config, path, sealed, fault)
    }

    pub fn closure(&mut self, config: &Config, reference: &Value) -> Result<BTreeSet<String>> {
        self.catalog.closure(config, reference)
    }

    pub fn retain(
        &mut self,
        config: &Config,
        owner: &str,
        refs: &[Value],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog.retain(config, owner, refs, fault)
    }

    pub(crate) fn hold(
        &mut self,
        config: &Config,
        owner: &str,
        refs: &[Value],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog.hold(config, owner, refs, fault)
    }

    pub fn retain_many(
        &mut self,
        config: &Config,
        roots: &[(String, Vec<Value>)],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog.retain_many(config, roots, fault)
    }

    pub(crate) fn retain_queue_many(
        &mut self,
        config: &Config,
        prefix: &str,
        roots: &[(String, Vec<Value>)],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog.retain_queue_many(config, prefix, roots, fault)
    }

    fn queue_reactivation_candidates(
        &mut self,
        config: &Config,
        prefix: &str,
        roots: &[(String, Value)],
    ) -> Result<Vec<Value>> {
        self.catalog
            .queue_reactivation_candidates(config, prefix, roots)
    }

    pub(crate) fn retain_queue_verified(
        &mut self,
        config: &Config,
        prefix: &str,
        verified: &VerifiedReactivation,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog
            .retain_queue_verified(config, prefix, verified, fault)
    }

    pub fn retain_reactivated(
        &mut self,
        config: &Config,
        roots: &[(String, Value)],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog.retain_reactivated(config, roots, fault)
    }

    fn reactivation_candidates(
        &mut self,
        config: &Config,
        roots: &[(String, Value)],
    ) -> Result<Vec<Value>> {
        self.catalog.reactivation_candidates(config, roots)
    }

    pub fn retain_verified(
        &mut self,
        config: &Config,
        verified: &VerifiedReactivation,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog.retain_verified(config, verified, fault)
    }

    pub fn release(
        &mut self,
        config: &Config,
        owners: &[String],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog.release(config, owners, fault)
    }

    pub fn require_retained(&self, owners: &HashSet<String>) -> Result<()> {
        self.catalog.require_retained(owners)
    }

    pub(crate) fn seal_unpublished(
        &mut self,
        config: &Config,
        path: &str,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog.seal_unpublished(config, path, fault)
    }

    pub fn consume(
        &mut self,
        config: &Config,
        refs: &[Value],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog.consume(config, refs, fault)
    }

    pub(crate) fn consume_once(
        &mut self,
        config: &Config,
        id: &str,
        refs: &[Value],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog.consume_once(config, id, refs, fault)
    }

    pub(crate) fn handoff_consumed(&self, id: &str) -> bool {
        self.catalog.handoff_consumed(id)
    }

    pub fn prune(
        &mut self,
        config: &Config,
        prefix: &str,
        live: &HashSet<String>,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog.prune(config, prefix, live, fault)
    }

    pub fn collect(&mut self, config: &Config, fault: &mut Fault<'_>) -> Result<Value> {
        self.catalog.collect(config, fault)
    }
    #[cfg(test)]
    pub(crate) fn collect_queue(
        &mut self,
        config: &Config,
        prepared: crate::queue_gc::Prepared<'_>,
        fault: &mut Fault<'_>,
    ) -> Result<Value> {
        self.catalog.collect_queue(config, prepared, fault)
    }
}

fn transaction_result<T>(
    result: std::result::Result<T, crate::catalog_transaction::Failure<Error>>,
) -> Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(crate::catalog_transaction::Failure::Effect(error)) => Err(error),
        Err(crate::catalog_transaction::Failure::Nested) => fail(
            "UnsafeRecovery",
            "Nested or interrupted catalog transaction; reopen if interrupted",
        ),
        Err(crate::catalog_transaction::Failure::Identity { expected, observed }) => {
            debug_assert_ne!(expected, observed);
            fail(
                "UnsafeRecovery",
                "Catalog log identity changed; lock does not cover the reopened file",
            )
        }
    }
}

struct NativeCall<F>(F);
impl<T, F: FnOnce(&mut LockedCatalog<'_>) -> Result<T>>
    crate::catalog_runtime::Call<crate::store::NamespaceSource> for NativeCall<F>
{
    type Output = T;
    fn call(self, catalog: &mut Catalog) -> Result<T> {
        (self.0)(&mut LockedCatalog { catalog })
    }
}
impl crate::catalog_gc::Library for crate::wal_file::NativeLibrary {
    fn queue_error(error: crate::gc_collection::QueueFailure<Error, Error, Error>) -> Error {
        queue_collection_error(error)
    }
}
impl crate::catalog_runtime::Library for crate::wal_file::NativeLibrary {
    fn binding_error() -> Error {
        Error::new(
            "InvalidReference",
            "Catalog configuration belongs to another storage pool or run",
        )
    }
    fn context_error() -> Error {
        Error::new(
            "UnsafeRecovery",
            "Catalog requires a fresh locked recovery transaction",
        )
    }
    fn replay_error(
        error: crate::wal_recovery::Failure<
            Error,
            crate::catalog_target::Error<crate::catalog_frame::Error<Error>>,
        >,
    ) -> Error {
        catalog_replay_error(error)
    }
}

/// Raw named-path operations for the collection adapter. The adapter
/// owns target indexing, barrier ordering, ancestor retries, and the receipt.
struct PhysicalIo<'a, S> {
    config: &'a Config,
    stat: S,
}
impl<S> crate::gc_collection::PhysicalModel for PhysicalIo<'_, S> {
    type Parent = PathBuf;
    type Path = PathBuf;
    type Directory = File;
}
impl crate::gc_io::IoError for std::io::Error {
    fn is_missing(&self) -> bool {
        self.kind() == std::io::ErrorKind::NotFound
    }
}
impl<S: FnMut(&Path) -> std::io::Result<std::fs::Metadata>> crate::gc_collection::Physical
    for PhysicalIo<'_, S>
{
    type Error = Error;
    fn resolve(&self, name: &String) -> Result<PathBuf> {
        self.config.path(name)
    }
    fn parent(&self, _name: &String, path: &PathBuf) -> PathBuf {
        path.parent().unwrap().to_path_buf()
    }
    fn stat(&mut self, path: &PathBuf) -> Result<Option<u64>> {
        Ok(crate::gc_io::classify((self.stat)(path))?.map(|metadata| metadata.len()))
    }
    fn remove(&mut self, _name: &String, path: &PathBuf) -> Result<bool> {
        Ok(crate::gc_io::classify(std::fs::remove_file(path))?.is_some())
    }
    fn open_parent(&mut self, parent: &PathBuf) -> Result<Option<File>> {
        Ok(crate::gc_io::classify(File::open(parent))?)
    }
    fn sync_parent(&mut self, directory: &File) -> Result<()> {
        directory.sync_all()?;
        Ok(())
    }
    fn prepare_parent(&mut self, parent: &PathBuf) -> Result<Vec<PathBuf>> {
        std::fs::create_dir_all(parent)?;
        let canonical = parent.canonicalize()?;
        Ok(canonical.ancestors().map(Path::to_path_buf).collect())
    }
    fn sync_ancestor(&mut self, parent: &PathBuf) -> Result<()> {
        File::open(parent)?.sync_all()?;
        Ok(())
    }
}

impl Catalog {
    pub fn open(config: &Config) -> Result<Self> {
        Self::open_with_create(config, true)
    }

    pub(crate) fn open_with_create(config: &Config, create: bool) -> Result<Self> {
        let pool_root = config
            .root
            .to_str()
            .ok_or_else(|| Error::new("ValueError", "Catalog root must be UTF-8"))?
            .to_string();
        crate::store::mkdir(&config.root, &mut |_| Ok(()))?;
        let opened = crate::catalog_primitives::open(&pool_root, create)?;
        File::open(&config.root)?.sync_all()?;
        Self::bind(
            opened,
            pool_root,
            config.run_id.clone(),
            crate::store::namespace_source(config.root.canonicalize()?),
        )
        .map_err(Into::into)
    }

    pub fn transaction<T>(
        &mut self,
        config: &Config,
        call: impl FnOnce(&mut LockedCatalog<'_>) -> Result<T>,
    ) -> Result<T> {
        // Keep the original lock's open description alive while reopening for
        // fresh I/O. The shared dispatcher checks identity before recovery.
        let result = self.execute::<crate::wal_file::NativeLibrary, _>(
            config.root.to_str(),
            &config.run_id,
            NativeCall(call),
        );
        transaction_result(result)
    }

    fn require_binding(&self, config: &Config) -> Result<()> {
        if !config.root.to_str().is_some_and(|root| {
            crate::identities::same_pool(&self.pool_root, &self.pool_run, root, &config.run_id)
        }) {
            return fail(
                "InvalidReference",
                "Catalog configuration belongs to another storage pool or run",
            );
        }
        Ok(())
    }

    fn require_context(&self, config: &Config) -> Result<()> {
        self.require_binding(config)?;
        self.require_ready()
    }

    fn require_ready(&self) -> Result<()> {
        if !self.transaction_active || !self.log.ready() {
            return fail(
                "UnsafeRecovery",
                "Catalog requires a fresh locked recovery transaction",
            );
        }
        Ok(())
    }

    #[cfg(test)]
    fn refresh_with_sync(
        &mut self,
        config: &Config,
        sync: impl FnOnce(&File) -> Result<()>,
    ) -> Result<()> {
        self.require_binding(config)?;
        let mut input = crate::wal_file::Source::new(&mut self.file, sync)?;
        let resolver = &mut self.namespace;
        let mut decoder =
            crate::catalog_frame::Decoder::new(&config.run_id, resolver, &mut self.closures);
        let mut target = crate::catalog_target::Apply::new(&mut self.log, &mut decoder);
        target.recover(&mut input).map_err(catalog_replay_error)?;
        Ok(())
    }

    fn identity(&mut self, _config: &Config, path: &str) -> Result<String> {
        self.namespace.identity(path)
    }

    fn validate_staging_edges(&self, additions: &HashMap<String, Vec<String>>) -> Result<()> {
        validate_staging_edges(&self.log.indexes().staging_edges, additions)
    }

    fn append(&mut self, config: &Config, mut event: Value, fault: &mut Fault<'_>) -> Result<()> {
        self.require_context(config)?;
        if event.as_object().is_none() {
            return fail("CorruptData", "Invalid catalog event");
        }
        // Certify only after normalizing every path-bearing field. A supplied
        // marker cannot bypass normalization on this writer path. Legacy logs
        // without the marker still resolve names when replayed.
        if let Some(owners) = event.get_mut("owners").and_then(Value::as_object_mut) {
            for paths in owners.values_mut().filter(|paths| !paths.is_null()) {
                let names: Vec<String> = serde_json::from_value(paths.clone())?;
                let resolved = names
                    .iter()
                    .map(|path| self.identity(config, path))
                    .collect::<Result<BTreeSet<_>>>()?;
                *paths = json!(resolved);
            }
        }
        if let Some(packs) = event.get_mut("packs").and_then(Value::as_object_mut) {
            let mut normalized = serde_json::Map::new();
            for (path, state) in packs.iter() {
                let key = self.identity(config, path)?;
                if !state.is_boolean() {
                    return fail("CorruptData", "Invalid pack state");
                }
                if let Some(previous) = normalized.insert(key, state.clone())
                    && previous != *state
                {
                    return fail("CorruptData", "Conflicting aliased pack states");
                }
            }
            *packs = normalized;
        }
        for field in ["deleted", "reclaimed"] {
            if let Some(paths) = event.get_mut(field) {
                let names: Vec<String> = serde_json::from_value(paths.clone())?;
                let resolved = names
                    .iter()
                    .map(|path| self.identity(config, path))
                    .collect::<Result<BTreeSet<_>>>()?;
                *paths = json!(resolved);
            }
        }
        event["path_encoding"] = json!(1);
        self.append_frame(config, event, fault)
    }

    fn append_frame(&mut self, config: &Config, event: Value, fault: &mut Fault<'_>) -> Result<()> {
        self.require_context(config)?;
        self.append_event::<crate::wal_file::NativeLibrary, _>(
            event,
            crate::wal_file::NativeHooks(fault),
        )
        .map_err(catalog_append_error)
    }

    pub fn pack(
        &mut self,
        config: &Config,
        path: &str,
        sealed: bool,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.require_context(config)?;
        let path = self.identity(config, path)?;
        if self.log.indexes().deleted.contains(&path) {
            return fail("InvalidReference", "Pack was reclaimed");
        }
        if self.log.indexes().packs.get(&path) == Some(&sealed) {
            return Ok(());
        }
        if self.log.indexes().packs.get(&path) == Some(&true) {
            return fail("InvalidReference", "Cannot reopen a sealed pack");
        }
        self.append(config, json!({"packs":{path:sealed}}), fault)
    }

    /// Only the writer of an unexposed extent or failed fresh setup calls this,
    /// after its private file has been durably removed. GC may have retired a
    /// seal whose response was lost; the permanent tombstone satisfies sealing.
    pub(crate) fn seal_unpublished(
        &mut self,
        config: &Config,
        path: &str,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.require_context(config)?;
        let path = self.identity(config, path)?;
        if self.log.indexes().deleted.contains(&path) {
            return Ok(());
        }
        self.pack(config, &path, true, fault)
    }

    pub fn closure(&mut self, config: &Config, reference: &Value) -> Result<BTreeSet<String>> {
        self.require_context(config)?;
        self.closure_inner(
            config,
            reference,
            &mut HashSet::new(),
            &mut Reader::new(config.clone()),
        )
    }

    fn closure_inner(
        &mut self,
        _config: &Config,
        reference: &Value,
        inspected: &mut HashSet<String>,
        reader: &mut Reader,
    ) -> Result<BTreeSet<String>> {
        let root_key = digest(reference)?;
        crate::closure::resolve(
            &mut ClosureReader {
                namespace: &mut self.namespace,
                inspected,
                reader,
                deleted: &self.log.indexes().deleted,
            },
            &mut self.closures,
            &self.log.indexes().deleted,
            root_key,
            reference.clone(),
        )
        .map_err(|error| match error {
            crate::closure::Failure::Read(error) => error,
            crate::closure::Failure::Reclaimed => Error::new(
                "InvalidReference",
                "Reference has been reclaimed; retain before releasing its source owner",
            ),
        })
    }

    pub fn retain(
        &mut self,
        config: &Config,
        owner: &str,
        refs: &[Value],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.retain_many(config, &[(owner.to_string(), refs.to_vec())], fault)
    }

    /// Preserve prior attempt roots even when a conflicting retry is rejected
    /// later. The publication releases this holder only after durable staging.
    pub(crate) fn hold(
        &mut self,
        config: &Config,
        owner: &str,
        refs: &[Value],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.require_context(config)?;
        if owner.is_empty() || owner.len() > 4096 {
            return fail("ValueError", "Invalid storage owner");
        }
        let mut additions = BTreeSet::new();
        let mut inspected = HashSet::new();
        let mut reader = Reader::new(config.clone());
        for reference in refs {
            additions.extend(self.closure_inner(config, reference, &mut inspected, &mut reader)?);
        }
        let mut paths = self
            .log
            .indexes()
            .owners
            .get(owner)
            .cloned()
            .unwrap_or_default();
        crate::holding::extend(&mut paths, &additions.into_iter().collect::<Vec<_>>());
        if paths.is_empty() || self.log.indexes().owners.get(owner) == Some(&paths) {
            return Ok(());
        }
        self.append(config, json!({"owners":{owner:paths}}), fault)
    }

    pub fn retain_many(
        &mut self,
        config: &Config,
        roots: &[(String, Vec<Value>)],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.require_context(config)?;
        writer_retirement::public_owners(roots.iter().map(|(owner, _)| owner.as_str()))?;
        self.retain_many_inner(config, roots, fault)
    }

    pub(crate) fn retain_queue_many(
        &mut self,
        config: &Config,
        prefix: &str,
        roots: &[(String, Vec<Value>)],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.require_context(config)?;
        writer_retirement::queue_owners(prefix, roots.iter().map(|(owner, _)| owner.as_str()))?;
        self.retain_many_inner(config, roots, fault)
    }

    fn retain_many_inner(
        &mut self,
        config: &Config,
        roots: &[(String, Vec<Value>)],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        let mut updates = serde_json::Map::new();
        let mut inspected = HashSet::new();
        let mut reader = Reader::new(config.clone());
        for (owner, refs) in roots {
            if owner.is_empty() || owner.len() > 4096 {
                return fail("ValueError", "Invalid storage owner");
            }
            let mut paths = BTreeSet::new();
            for r in refs {
                paths.extend(self.closure_inner(config, r, &mut inspected, &mut reader)?);
            }
            if paths.is_empty() && !self.log.indexes().owners.contains_key(owner) {
                continue;
            }
            if self.log.indexes().owners.get(owner) != Some(&paths) {
                updates.insert(owner.clone(), json!(paths));
            }
        }
        if updates.is_empty() {
            return Ok(());
        }
        self.append(config, json!({"owners":updates}), fault)
    }

    /// Lower-level compatibility path: the caller's transaction also covers
    /// payload verification. Prefer Config::retain_reactivated to release the
    /// catalog lock during payload I/O.
    pub fn retain_reactivated(
        &mut self,
        config: &Config,
        roots: &[(String, Value)],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        let missing = self.reactivation_candidates(config, roots)?;
        let verified = VerifiedReactivation::verify(config, roots, missing, fault)?;
        self.retain_verified(config, &verified, fault)
    }

    fn reactivation_candidates(
        &mut self,
        config: &Config,
        roots: &[(String, Value)],
    ) -> Result<Vec<Value>> {
        self.require_context(config)?;
        writer_retirement::public_owners(roots.iter().map(|(owner, _)| owner.as_str()))?;
        self.reactivation_candidates_inner(config, roots)
    }

    fn queue_reactivation_candidates(
        &mut self,
        config: &Config,
        prefix: &str,
        roots: &[(String, Value)],
    ) -> Result<Vec<Value>> {
        self.require_context(config)?;
        writer_retirement::queue_owners(prefix, roots.iter().map(|(owner, _)| owner.as_str()))?;
        self.reactivation_candidates_inner(config, roots)
    }

    /// Reusing an owner is safe only when it already covers this root. Queue
    /// owners bind the immutable reference digest in their reserved name; public
    /// names are mutable, so compare their actual retained pack closure.
    fn check_reactivation_owners(
        &mut self,
        config: &Config,
        roots: &[(String, Value)],
    ) -> Result<()> {
        let mut reader = Reader::new(config.clone());
        let mut inspected = HashSet::new();
        for (owner, reference) in roots {
            if !self.log.indexes().owners.contains_key(owner) {
                continue;
            }
            let matches = if owner.starts_with("queue:") {
                owner.rsplit_once(':').map(|(_, key)| key) == Some(digest(reference)?.as_str())
            } else {
                let requested =
                    self.closure_inner(config, reference, &mut inspected, &mut reader)?;
                !requested.is_empty() && requested.is_subset(&self.log.indexes().owners[owner])
            };
            if !matches {
                return fail(
                    "UnsafeRecovery",
                    "Existing owner does not protect the requested root",
                );
            }
        }
        Ok(())
    }

    fn reactivation_candidates_inner(
        &mut self,
        config: &Config,
        roots: &[(String, Value)],
    ) -> Result<Vec<Value>> {
        self.check_reactivation_owners(config, roots)?;
        let missing = roots
            .iter()
            .filter(|(owner, _)| !self.log.indexes().owners.contains_key(owner))
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(Vec::new());
        }
        let protected = self.log.indexes().roots();
        let mut candidates = Vec::new();
        let mut reader = Reader::new(config.clone());
        let mut inspected = HashSet::new();
        for (_, reference) in missing {
            let paths = self.closure_inner(config, reference, &mut inspected, &mut reader)?;
            if paths.is_empty() || paths.iter().any(|p| !protected.contains(p)) {
                return fail(
                    "UnsafeRecovery",
                    "Restored queue root has no durable source owner",
                );
            }
            candidates.push(reference.clone());
        }
        Ok(candidates)
    }

    /// Must run within a fresh catalog transaction. Authentication is not a
    /// retained snapshot: owners/tombstones are checked again before any append.
    pub fn retain_verified(
        &mut self,
        config: &Config,
        verified: &VerifiedReactivation,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.require_context(config)?;
        writer_retirement::public_owners(verified.roots.iter().map(|(owner, _)| owner.as_str()))?;
        self.retain_verified_inner(config, verified, fault)
    }

    pub(crate) fn retain_queue_verified(
        &mut self,
        config: &Config,
        prefix: &str,
        verified: &VerifiedReactivation,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.require_context(config)?;
        writer_retirement::queue_owners(
            prefix,
            verified.roots.iter().map(|(owner, _)| owner.as_str()),
        )?;
        self.retain_verified_inner(config, verified, fault)
    }

    fn retain_verified_inner(
        &mut self,
        config: &Config,
        verified: &VerifiedReactivation,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        if verified.root != config.root || verified.run_id != config.run_id {
            return fail(
                "InvalidReference",
                "Verified reactivation belongs to another storage pool",
            );
        }
        self.check_reactivation_owners(config, &verified.roots)?;
        let missing = verified
            .roots
            .iter()
            .filter(|(owner, _)| !self.log.indexes().owners.contains_key(owner))
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(());
        }
        let protected = self.log.indexes().roots();
        let mut updates = serde_json::Map::new();
        let mut reader = Reader::new(config.clone());
        let mut inspected = HashSet::new();
        for (owner, reference) in missing {
            let paths = self.closure_inner(config, reference, &mut inspected, &mut reader)?;
            let statuses = paths
                .iter()
                .map(|p| PackStatus {
                    sealed: false,
                    owned: protected.contains(p),
                    tombstoned: self.log.indexes().deleted.contains(p),
                })
                .collect::<Vec<_>>();
            if !protocol::reactivation_permitted(
                verified.verified.contains(&digest(reference)?),
                &statuses,
            ) {
                return fail(
                    "UnsafeRecovery",
                    "Restored queue root lost verification or durable source ownership; retry restore",
                );
            }
            updates.insert(owner.clone(), json!(paths));
        }
        if updates.is_empty() {
            return Ok(());
        }
        self.append(config, json!({"owners":updates}), fault)
    }

    pub fn release(
        &mut self,
        config: &Config,
        owners: &[String],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.require_context(config)?;
        writer_retirement::public_owners(owners.iter().map(String::as_str))?;
        let mut resolver = &mut self.namespace;
        let mut writer = crate::wal_file::Appender::new(&mut self.file, Kind::Catalog, fault);
        crate::queue_gc::release(
            &mut self.log,
            &mut resolver,
            &config.run_id,
            owners,
            &mut writer,
        )
        .map_err(catalog_append_error)
    }

    pub fn require_retained(&self, owners: &HashSet<String>) -> Result<()> {
        self.require_ready()?;
        crate::queue_gc::require(
            &self.log.indexes().owners,
            &self.log.indexes().deleted,
            owners,
        )
        .map_err(|error| match error {
            crate::queue_gc::OwnershipError::Missing => Error::new(
                "UnsafeRecovery",
                "Live queue root has no durable storage owner",
            ),
            crate::queue_gc::OwnershipError::Invalid => Error::new(
                "UnsafeRecovery",
                "Live queue root has invalid storage ownership",
            ),
        })
    }

    pub fn staged(reference: &Value) -> Result<String> {
        Ok(format!("staged:{}", digest(reference)?))
    }

    pub fn consume(
        &mut self,
        config: &Config,
        refs: &[Value],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.consume_with_receipt(config, refs, None, fault)
    }

    // Queue WAL markers name a single adoption. Unlike explicit publication
    // release, retrying it must not consume staging reestablished afterwards.
    pub(crate) fn consume_once(
        &mut self,
        config: &Config,
        id: &str,
        refs: &[Value],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.require_context(config)?;
        if !valid_handoff_id(id) {
            return fail("InvalidReference", "Invalid staging handoff identity");
        }
        if self.log.indexes().consumed_handoffs.contains(id) {
            return Ok(());
        }
        self.consume_with_receipt(config, refs, Some(id), fault)
    }

    pub(crate) fn handoff_consumed(&self, id: &str) -> bool {
        self.log.indexes().consumed_handoffs.contains(id)
    }

    fn consume_with_receipt(
        &mut self,
        config: &Config,
        refs: &[Value],
        receipt: Option<&str>,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.require_context(config)?;
        let plan = crate::staging_consumption::plan(
            &mut StagingReader {
                reader: Reader::new(config.clone()),
                fault,
            },
            &mut ReferenceDigest,
            &self.log.indexes().staging_edges,
            refs,
        )
        .map_err(staging_error)?;
        self.validate_staging_edges(&plan.additions)?;
        let Some(event) =
            crate::staging_consumption::value(plan, &self.log.indexes().owners, receipt)
        else {
            return Ok(());
        };
        // The same frame contains exact direct edges, owner releases and the
        // consumption receipt, so retries still work after manifests are GC'd.
        self.append(config, event, fault)
    }

    pub fn prune(
        &mut self,
        config: &Config,
        prefix: &str,
        live: &HashSet<String>,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.require_context(config)?;
        let dead = crate::queue_gc::prunable(&self.log.indexes().owners, prefix, live);
        self.release(config, &dead, fault)
    }

    pub fn collect(&mut self, config: &Config, fault: &mut Fault<'_>) -> Result<Value> {
        self.collect_with_stat(config, fault, |path| std::fs::metadata(path))
    }

    fn collect_with_stat(
        &mut self,
        config: &Config,
        fault: &mut Fault<'_>,
        stat: impl FnMut(&std::path::Path) -> std::io::Result<std::fs::Metadata>,
    ) -> Result<Value> {
        self.require_context(config)?;
        let result = self
            .collect_native::<crate::wal_file::NativeLibrary, _, _>(
                crate::wal_file::NativeHooks(fault),
                PhysicalIo { config, stat },
            )
            .map_err(collection_error)?;
        Ok(self.collection_summary(result))
    }

    pub(crate) fn collect_queue_transaction(
        &mut self,
        config: &Config,
        prepared: crate::queue_gc::Prepared<'_>,
        fault: &mut Fault<'_>,
    ) -> Result<Value> {
        let call = crate::catalog_gc::Queue::<crate::wal_file::NativeLibrary, _, _>::new(
            prepared,
            crate::wal_file::NativeHooks(fault),
            PhysicalIo {
                config,
                stat: |path: &Path| std::fs::metadata(path),
            },
        );
        let result = self.execute::<crate::wal_file::NativeLibrary, _>(
            config.root.to_str(),
            &config.run_id,
            call,
        );
        Ok(self.collection_summary(transaction_result(result)?))
    }

    fn collection_summary(&self, result: crate::gc_collection::Collected) -> Value {
        json!({"reclaimed_files":result.statistics.files,"reclaimed_bytes":result.statistics.bytes,"owners":self.log.indexes().owners.len(),
            "open_packs":self.log.indexes().packs.iter().filter(|(p,s)| !**s && !self.log.indexes().deleted.contains(*p)).count(),
            "retained_packs":result.retained,"tombstones":self.log.indexes().deleted.len()})
    }

    #[cfg(test)]
    fn collect_queue(
        &mut self,
        config: &Config,
        prepared: crate::queue_gc::Prepared<'_>,
        fault: &mut Fault<'_>,
    ) -> Result<Value> {
        self.require_context(config)?;
        let result = self
            .collect_queue_native::<crate::wal_file::NativeLibrary, _, _>(
                prepared,
                crate::wal_file::NativeHooks(fault),
                PhysicalIo {
                    config,
                    stat: |path: &Path| std::fs::metadata(path),
                },
            )
            .map_err(queue_collection_error)?;
        Ok(self.collection_summary(result))
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use crate::store::{Record, Store};

    #[test]
    fn cold_closure_rejects_reclaimed_roots_and_dependencies_before_payload_reads() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut config = Config::new(
            directory.path().to_str().unwrap(),
            "cold-tombstones",
            vec!["bytes.v1".into()],
            1024 * 1024,
            4 * 1024 * 1024,
            0,
        )?;
        config.online_gc = true;
        let mut writer = Store::new(config.clone());
        let child = writer.publish(
            &json!([{"records":[0],"dependencies":[]}]),
            vec![Record {
                metadata: json!({"record_id":"child","codec":"bytes.v1","metadata":{},"tokens":0}),
                payload: b"payload".as_slice().into(),
            }],
            "child",
            &mut |_| Ok(()),
        )?[0]
            .clone();
        let record = config.validate(&child, None, None)?[0].clone();
        writer.close()?;
        let mut parent_writer = Store::new(config.clone());
        let parent = parent_writer.publish(
            &json!([{"records":[],"dependencies":[child.clone()]}]),
            Vec::new(),
            "parent",
            &mut |_| Ok(()),
        )?[0]
            .clone();
        parent_writer.close()?;
        let paths = BTreeSet::from([
            field(&child["manifest"]["segment"], "path")?.to_owned(),
            field(&record["segment"], "path")?.to_owned(),
        ]);
        config.catalog(|c| {
            c.consume(&config, &[parent.clone(), child.clone()], &mut |_| Ok(()))?;
            for path in &paths {
                assert!(
                    !c.catalog
                        .log
                        .indexes()
                        .owners
                        .values()
                        .any(|owned| owned.contains(path))
                );
            }
            c.catalog
                .append(&config, json!({"deleted":paths}), &mut |_| Ok(()))
        })?;
        for path in &paths {
            std::fs::remove_file(config.root.join(path))?;
        }
        // A fresh catalog has no derived closure or namespace cache. The parent
        // manifest still exists, while its deleted dependency is permanently bad.
        let mut catalog = Catalog::open(&config)?;
        catalog.transaction(&config, |c| {
            for reference in [&child, &record, &parent] {
                let error = c.closure(&config, reference).unwrap_err();
                assert_eq!(error.kind, "InvalidReference");
                assert!(error.message.contains("reclaimed"), "{error:?}");
            }
            Ok(())
        })
    }

    #[test]
    fn append_certifies_only_after_normalizing_all_path_fields() -> Result<()> {
        let private = tempfile::tempdir()?;
        let config = Config::new(
            private.path().to_str().unwrap(),
            "encoding",
            vec![],
            1024,
            1024,
            1024,
        )?;
        std::fs::write(private.path().join("real.pack"), b"data")?;
        std::os::unix::fs::symlink("real.pack", private.path().join("alias.pack"))?;
        std::os::unix::fs::symlink("../escape", private.path().join("escape.pack"))?;
        let mut catalog = Catalog::open(&config)?;
        catalog.transaction(&config, |c| {
            // Even a supplied marker does not skip writer-side normalization.
            c.catalog.append(
                &config,
                json!({"path_encoding":1,"owners":{"reader":["alias.pack"]},"packs":{"alias.pack":true}}),
                &mut |_| Ok(()),
            )?;
            assert_eq!(c.catalog.log.indexes().owners["reader"], BTreeSet::from(["real.pack".into()]));
            let before = std::fs::read(config.root.join("storage.log"))?;
            let error = c.catalog.append(
                &config,
                json!({"path_encoding":1,"owners":{"bad":["escape.pack"]}}),
                &mut |_| panic!("invalid identity must fail before catalog I/O"),
            ).unwrap_err();
            assert_eq!(error.kind, "InvalidReference");
            assert_eq!(std::fs::read(config.root.join("storage.log"))?, before);
            c.release(&config, &["reader".into()], &mut |_| Ok(()))?;
            std::fs::remove_file(config.root.join("real.pack"))?;
            File::open(&config.root)?.sync_all()?;
            c.catalog.append(
                &config,
                json!({"deleted":["alias.pack"],"reclaimed":["alias.pack"]}),
                &mut |_| Ok(()),
            )
        })?;
        let bytes = std::fs::read(config.root.join("storage.log"))?;
        let mut offset = 0;
        while offset < bytes.len() {
            let n =
                u64::from_le_bytes(bytes[offset + 16..offset + 24].try_into().unwrap()) as usize;
            let event: Value = serde_json::from_slice(&bytes[offset + 56..offset + 56 + n])?;
            assert_eq!(event["path_encoding"], 1);
            assert!(!event.to_string().contains("alias.pack"));
            offset += 96 + n;
        }
        let mut fresh = Catalog::open(&config)?;
        fresh.transaction(&config, |c| {
            assert!(c.catalog.log.indexes().deleted.contains("real.pack"));
            assert!(c.catalog.log.indexes().pending_deletions.is_empty());
            assert!(c.catalog.log.indexes().owners.is_empty());
            Ok(())
        })
    }

    #[test]
    fn replay_normalizes_legacy_owners_packs_tombstones_and_receipts() -> Result<()> {
        let private = tempfile::tempdir()?;
        let make_config = || -> Result<Config> {
            let mut config = Config::new(
                private.path().to_str().unwrap(),
                "identity",
                vec!["bytes.v1".into()],
                1024,
                1024,
                1024,
            )?;
            config.online_gc = true;
            Ok(config)
        };
        let config = make_config()?;
        let mut writer = Store::new(config.clone());
        let refs = writer.write(
            &[Record {
                metadata: json!({"record_id":"r", "codec":"bytes.v1", "metadata":{}, "tokens":0}),
                payload: b"data".as_slice().into(),
            }],
            "source",
            &mut |_| Ok(()),
        )?;
        let original = field(&refs[0]["segment"], "path")?.to_string();
        let path = std::path::Path::new(&original);
        std::os::unix::fs::symlink(&original, private.path().join("reader.pack"))?;
        std::os::unix::fs::symlink(path.parent().unwrap(), private.path().join("writer"))?;
        let legacy_pack = format!("writer/{}", path.file_name().unwrap().to_str().unwrap());
        // append_frame writes these legacy spellings verbatim. apply/replay must
        // resolve all four event fields, not merely new retain calls.
        config.catalog(|c| {
            c.catalog.append_frame(
                &config,
                json!({"owners":{"legacy":["reader.pack"]}, "packs":{&legacy_pack:true}}),
                &mut |_| Ok(()),
            )
        })?;
        writer.seal(&mut |_| Ok(()))?;
        config.catalog(|c| c.consume(&config, &refs, &mut |_| Ok(())))?;
        drop(writer);
        drop(config);
        let config = make_config()?;
        config.catalog(|c| {
            assert_eq!(
                c.catalog.log.indexes().owners["legacy"],
                BTreeSet::from([original.clone()])
            );
            assert_eq!(c.collect(&config, &mut |_| Ok(()))?["reclaimed_files"], 0);
            c.release(&config, &["legacy".into()], &mut |_| Ok(()))?;
            assert_eq!(c.collect(&config, &mut |_| Ok(()))?["reclaimed_files"], 1);
            c.catalog.append_frame(
                &config,
                json!({"deleted":[&legacy_pack],"reclaimed":[&legacy_pack]}),
                &mut |_| Ok(()),
            )
        })?;
        drop(config);
        let config = make_config()?;
        config.catalog(|c| {
            assert!(c.catalog.log.indexes().deleted.contains(&original));
            assert!(c.catalog.log.indexes().pending_deletions.is_empty());
            assert!(c.catalog.log.indexes().packs.is_empty());
            assert_eq!(c.collect(&config, &mut |_| Ok(()))?["reclaimed_files"], 0);
            Ok(())
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    use crate::bytes;
    use std::fs;
    use std::io::Seek;

    #[test]
    fn append_and_fresh_replay_agree_on_every_business_index() -> Result<()> {
        let (_directory, config, mut catalog) = setup(false)?;
        let nodes: Vec<_> = (0..16).map(|i| format!("staged:{i:064x}")).collect();
        let mut edges = HashMap::with_capacity(64);
        for node in nodes.iter().rev() {
            edges.insert(node.clone(), vec![nodes[0].clone(), nodes[1].clone()]);
        }
        let receipt = "a".repeat(64);
        for event in [
            json!({"staging_edges":edges, "owners":{"reader/α":["raw/pack", "raw/数据", "raw/pack"]},
                "packs":{"raw/pack":true,"raw/数据":true}}),
            json!({"owners":{"source":null,"reader/α":null},"consumed_handoffs":[&receipt,&receipt]}),
            json!({"deleted":["raw/pack","raw/数据","raw/pack"],"reclaimed":["raw/pack"]}),
            json!({"reclaimed":["raw/数据"]}),
        ] {
            catalog.transaction(&config, |c| {
                c.catalog.append(&config, event, &mut |_| Ok(()))
            })?;
            let expected = catalog.log.indexes();
            let bytes = fs::read(config.root.join("storage.log"))?;
            // A new catalog creates new parsed Values, Vec allocations and hash
            // seeds. Compare sets/maps, not a particular iteration order.
            let mut recovered = Catalog::open(&config)?;
            recovered.transaction(&config, |c| {
                let actual = c.catalog.log.indexes();
                assert_eq!(actual.owners, expected.owners);
                assert_eq!(actual.packs, expected.packs);
                assert_eq!(actual.deleted, expected.deleted);
                assert_eq!(actual.pending_deletions, expected.pending_deletions);
                assert_eq!(actual.staging_edges, expected.staging_edges);
                assert_eq!(actual.consumed_handoffs, expected.consumed_handoffs);
                assert_eq!(c.catalog.log.offset(), catalog.log.offset());
                assert_eq!(c.catalog.log.sequence(), catalog.log.sequence());
                assert_eq!(c.catalog.log.floor(), catalog.log.floor());
                Ok(())
            })?;
            assert_eq!(fs::read(config.root.join("storage.log"))?, bytes);
        }
        Ok(())
    }

    #[test]
    fn append_rejects_invalid_events_before_wal_side_effects() -> Result<()> {
        for event in [
            json!(false),
            json!([]),
            json!({"owners": false}),
            json!({"owners":{"source":null}, "consumed_handoffs":["invalid"]}),
            json!({"owners":{"tentative":["raw/pack"]}, "reclaimed":["raw/not-marked"]}),
            json!({"path_encoding": 2}),
            json!({"path_encoding": 1, "packs":{"../outside":true}}),
            json!({"staging_edges":{"invalid":[]}}),
        ] {
            let (_directory, config, mut catalog) = setup(false)?;
            let before = fs::read(config.root.join("storage.log"))?;
            catalog.transaction(&config, |c| {
                let sequence = c.catalog.log.sequence();
                let offset = c.catalog.log.offset();
                let mut effects = Vec::new();
                if event.as_object().is_none() {
                    assert!(
                        c.catalog
                            .append(&config, event.clone(), &mut |at| {
                                effects.push(at.to_string());
                                Ok(())
                            })
                            .is_err()
                    );
                }
                assert!(
                    c.catalog
                        .append_frame(&config, event, &mut |at| {
                            effects.push(at.to_string());
                            Ok(())
                        })
                        .is_err()
                );
                assert!(
                    effects.is_empty(),
                    "invalid event reached WAL effects: {effects:?}"
                );
                assert_eq!(fs::read(config.root.join("storage.log"))?, before);
                assert_eq!(c.catalog.log.sequence(), sequence);
                assert_eq!(c.catalog.log.offset(), offset);
                assert!(!c.catalog.log.dirty());
                assert!(!c.catalog.log.indexes().owners.contains_key("tentative"));
                c.require_retained(&HashSet::from(["source".into()]))?;
                c.pack(&config, "raw/valid-next", true, &mut |_| Ok(()))
            })?;
            let mut recovered = Catalog::open(&config)?;
            recovered.transaction(&config, |c| {
                assert_eq!(
                    c.catalog.log.indexes().packs.get("raw/valid-next"),
                    Some(&true)
                );
                c.require_retained(&HashSet::from(["source".into()]))
            })?;
        }
        Ok(())
    }

    #[test]
    fn transaction_error_releases_lock_and_preserves_durable_callback_effects() -> Result<()> {
        let (_directory, config, mut catalog) = setup(false)?;
        let error = catalog
            .transaction(&config, |c| -> Result<()> {
                c.pack(&config, "raw/committed-before-error", true, &mut |_| Ok(()))?;
                let contender = File::open(config.root.join("storage.log"))?;
                assert!(contender.try_lock().is_err());
                Err(Error::new("CallbackFailed", "durable effect before error"))
            })
            .unwrap_err();
        assert_eq!(error.kind, "CallbackFailed");
        assert!(!catalog.transaction_active);
        let contender = File::open(config.root.join("storage.log"))?;
        contender.try_lock().unwrap();
        contender.unlock()?;
        catalog.transaction(&config, |c| {
            assert_eq!(
                c.catalog
                    .log
                    .indexes()
                    .packs
                    .get("raw/committed-before-error"),
                Some(&true)
            );
            Ok(())
        })
    }

    #[test]
    fn caught_callback_unwind_requires_reopen_and_preserves_committed_effects() -> Result<()> {
        let (_directory, config, mut catalog) = setup(false)?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            catalog.transaction(&config, |c| -> Result<()> {
                c.pack(&config, "raw/committed-before-panic", true, &mut |_| Ok(()))?;
                panic!("callback interrupted after a durable effect");
            })
        }));
        assert!(outcome.is_err());
        assert!(catalog.transaction_active);
        let before = fs::read(config.root.join("storage.log"))?;
        assert_eq!(
            catalog
                .pack(&config, "raw/after-panic", true, &mut |_| Ok(()))
                .unwrap_err()
                .kind,
            "UnsafeRecovery"
        );
        assert_eq!(fs::read(config.root.join("storage.log"))?, before);
        assert_eq!(
            catalog.transaction(&config, |_| Ok(())).unwrap_err().kind,
            "UnsafeRecovery"
        );
        let mut reopened = Catalog::open(&config)?;
        reopened.transaction(&config, |c| {
            assert_eq!(
                c.catalog
                    .log
                    .indexes()
                    .packs
                    .get("raw/committed-before-panic"),
                Some(&true)
            );
            Ok(())
        })
    }

    #[test]
    fn transaction_cannot_reopen_a_different_pool_under_the_original_lock() -> Result<()> {
        let (_directory, config, mut catalog) = setup(false)?;
        let other = tempfile::tempdir()?;
        let mut peer = config.clone();
        peer.root = other.path().to_path_buf();
        let original = fs::read(config.root.join("storage.log"))?;
        fs::write(peer.root.join("storage.log"), &original)?;
        let result = catalog.transaction(&peer, |c| {
            c.release(&peer, &["source".into()], &mut |_| Ok(()))
        });
        assert!(
            result.is_err(),
            "catalog switched log objects while holding the original pool lock"
        );
        assert_eq!(fs::read(config.root.join("storage.log"))?, original);
        assert_eq!(fs::read(peer.root.join("storage.log"))?, original);
        catalog.transaction(&config, |c| {
            c.require_retained(&HashSet::from(["source".into()]))
        })
    }

    #[test]
    fn transaction_rejects_replaced_log_before_replay_or_callback() -> Result<()> {
        let (_directory, config, mut catalog) = setup(false)?;
        let path = config.root.join("storage.log");
        let original = fs::read(&path)?;
        fs::rename(&path, config.root.join("detached.log"))?;
        fs::write(&path, &original)?;
        let mut called = false;
        let error = catalog
            .transaction(&config, |_| {
                called = true;
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error.kind, "UnsafeRecovery");
        assert!(!called);
        assert_eq!(fs::read(&path)?, original);
        assert_eq!(fs::read(config.root.join("detached.log"))?, original);
        Ok(())
    }

    #[test]
    fn active_catalog_rejects_foreign_configuration_before_cached_or_io_success() -> Result<()> {
        let (_directory, config, mut catalog) = setup(false)?;
        let other = tempfile::tempdir()?;
        for wrong_root in [false, true] {
            let mut foreign = config.clone();
            if wrong_root {
                foreign.root = other.path().to_path_buf();
            } else {
                foreign.run_id = "foreign-run".into();
            }
            let original = fs::read(config.root.join("storage.log"))?;
            catalog.transaction(&config, |c| {
                assert_eq!(
                    c.pack(&foreign, "raw/pack", false, &mut |_| Ok(()))
                        .unwrap_err()
                        .kind,
                    "InvalidReference"
                );
                assert_eq!(
                    c.release(&foreign, &[], &mut |_| Ok(())).unwrap_err().kind,
                    "InvalidReference"
                );
                assert_eq!(
                    c.collect(&foreign, &mut |_| Ok(())).unwrap_err().kind,
                    "InvalidReference"
                );
                c.require_retained(&HashSet::from(["source".into()]))
            })?;
            assert_eq!(fs::read(config.root.join("storage.log"))?, original);
        }
        Ok(())
    }

    #[test]
    fn uncertain_append_cannot_be_followed_by_another_write_in_the_same_transaction() -> Result<()>
    {
        let (_directory, config, mut catalog) = setup(false)?;
        let result = catalog.transaction(&config, |c| {
            let failed = c.pack(&config, "raw/uncertain", true, &mut |at| {
                if at == "after_storage_catalog_part" {
                    return Err(Error::new("StorageUnavailable", "injected partial append"));
                }
                Ok(())
            });
            assert_eq!(failed.unwrap_err().kind, "StorageUnavailable");
            let uncertain_size = c.catalog.file.metadata()?.len();
            assert_eq!(
                c.pack(&config, "raw/pack", false, &mut |_| Ok(()))
                    .unwrap_err()
                    .kind,
                "UnsafeRecovery"
            );
            assert_eq!(
                c.require_retained(&HashSet::from(["source".into()]))
                    .unwrap_err()
                    .kind,
                "UnsafeRecovery"
            );
            let later = c.pack(&config, "raw/later", true, &mut |_| Ok(()));
            assert!(
                matches!(later, Err(ref e) if e.kind == "UnsafeRecovery"),
                "catalog accepted another write after an uncertain append"
            );
            assert_eq!(c.catalog.file.metadata()?.len(), uncertain_size);
            // Catching the errors must not turn the enclosing transaction into
            // a successful confirmation of an uncertain catalog state.
            Ok(())
        });
        assert!(matches!(result, Err(ref e) if e.kind == "UnsafeRecovery"));
        catalog.transaction(&config, |c| {
            assert!(!c.catalog.log.indexes().packs.contains_key("raw/uncertain"));
            c.pack(&config, "raw/later", true, &mut |_| Ok(()))
        })?;
        let mut reopened = Catalog::open(&config)?;
        reopened.transaction(&config, |c| {
            assert_eq!(c.catalog.log.indexes().packs.get("raw/later"), Some(&true));
            assert!(!c.catalog.log.indexes().packs.contains_key("raw/uncertain"));
            Ok(())
        })
    }

    #[test]
    fn catalog_methods_require_a_live_transaction_and_nested_entry_is_rejected() -> Result<()> {
        let (_directory, config, mut catalog) = setup(false)?;
        let before = fs::read(config.root.join("storage.log"))?;
        assert_eq!(
            catalog
                .pack(&config, "raw/pack", false, &mut |_| Ok(()))
                .unwrap_err()
                .kind,
            "UnsafeRecovery"
        );
        assert_eq!(
            catalog
                .require_retained(&HashSet::from(["source".into()]))
                .unwrap_err()
                .kind,
            "UnsafeRecovery"
        );
        assert_eq!(
            catalog.collect(&config, &mut |_| Ok(())).unwrap_err().kind,
            "UnsafeRecovery"
        );
        assert_eq!(fs::read(config.root.join("storage.log"))?, before);
        catalog.transaction(&config, |c| {
            assert_eq!(
                c.catalog.transaction(&config, |_| Ok(())).unwrap_err().kind,
                "UnsafeRecovery"
            );
            // Rejecting a nested entry has no I/O effect and leaves the valid
            // outer scope available for ordinary operations.
            c.require_retained(&HashSet::from(["source".into()]))?;
            c.pack(&config, "raw/later", true, &mut |_| Ok(()))
        })?;
        assert!(!catalog.transaction_active);
        Ok(())
    }

    #[test]
    fn failed_gc_hooks_with_written_tail_quarantine_until_fresh_recovery() -> Result<()> {
        for point in [
            "after_gc_tombstone",
            "before_gc_receipt",
            "before_directory_sync",
        ] {
            let (_directory, config, mut catalog) = setup(false)?;
            let pack = config.path("raw/pack")?;
            fs::create_dir_all(pack.parent().unwrap())?;
            fs::write(&pack, b"payload")?;
            catalog.transaction(&config, |c| {
                c.release(&config, &["source".to_owned()], &mut |_| Ok(()))?;
                c.pack(&config, "raw/pack", true, &mut |_| Ok(()))
            })?;
            let log_path = config.root.join("storage.log");
            let mut injected = false;
            let error = catalog
                .transaction(&config, |c| {
                    let error = c
                        .collect(&config, &mut |at| {
                            if point == "before_directory_sync" && at == "after_gc_tombstone" {
                                fs::remove_dir_all(pack.parent().unwrap())?;
                            }
                            if at == point && !injected {
                                injected = true;
                                fs::OpenOptions::new()
                                    .append(true)
                                    .open(&log_path)?
                                    .write_all(&framing::magic(Kind::Catalog)[..4])?;
                                return Err(std::io::Error::from_raw_os_error(5).into());
                            }
                            Ok(())
                        })
                        .unwrap_err();
                    assert!(injected, "{point}");
                    assert!(
                        !c.catalog.log.ready(),
                        "failed hook must quarantine {point}"
                    );
                    let mut entered_append = false;
                    let blocked = c
                        .pack(&config, "raw/after-error", true, &mut |_| {
                            entered_append = true;
                            Ok(())
                        })
                        .unwrap_err();
                    assert_eq!(blocked.kind, "UnsafeRecovery");
                    assert!(
                        !entered_append,
                        "quarantine must precede another writer hook"
                    );
                    Err::<(), _>(error)
                })
                .unwrap_err();
            assert_eq!(error.kind, "StorageUnavailable");
            assert!(fs::metadata(&log_path)?.len() > catalog.log.offset());
            catalog.transaction(&config, |c| {
                c.collect(&config, &mut |_| Ok(()))?;
                assert!(c.catalog.log.indexes().pending_deletions.is_empty());
                assert!(c.catalog.log.indexes().deleted.contains("raw/pack"));
                assert!(!pack.exists());
                c.pack(&config, "raw/after-retry", true, &mut |_| Ok(()))
            })?;
            assert!(catalog.log.ready());
            assert_eq!(catalog.file.stream_position()?, catalog.log.offset());
        }
        Ok(())
    }

    fn setup(receipt: bool) -> Result<(tempfile::TempDir, Config, Catalog)> {
        let directory = tempfile::tempdir()?;
        let config = Config::new(
            directory.path().to_str().unwrap(),
            "replay-test",
            vec!["bytes.v1".into()],
            1024,
            1024,
            1024,
        )?;
        let mut catalog = Catalog::open(&config)?;
        catalog.transaction(&config, |c| {
            c.catalog.append(&config, json!({"packs":{"raw/pack":false},"owners":{"source":["raw/pack"]}}), &mut |_| Ok(()))?;
            if receipt {
                c.catalog.append(&config, json!({"owners":{"source":null},"packs":{"raw/pack":true},"deleted":["raw/pack"]}), &mut |_| Ok(()))?;
            }
            Ok(())
        })?;
        Ok((directory, config, catalog))
    }

    fn uncertain(catalog: &mut Catalog, config: &Config, event: Value) -> Result<()> {
        // Write a valid frame and fail its synchronization response.
        let error = catalog
            .transaction(config, |catalog| {
                catalog.catalog.append_frame(config, event, &mut |at| {
                    if at == "before_storage_catalog_sync" {
                        return Err(std::io::Error::from_raw_os_error(5).into());
                    }
                    Ok(())
                })
            })
            .unwrap_err();
        assert_eq!(error.kind, "StorageUnavailable");
        Ok(())
    }

    fn inject_invalid_unconfirmed_frame(
        catalog: &mut Catalog,
        config: &Config,
        mut event: Value,
    ) -> Result<()> {
        // Offline-format fault fixture: bypass the writer admission check to
        // exercise recovery of a previously produced, unconfirmed bad frame.
        let error = catalog
            .transaction(config, |c| {
                event["run_id"] = json!(config.run_id);
                let raw = bytes(&event)?;
                let prefix =
                    framing::prefix(Kind::Catalog, c.catalog.log.sequence(), raw.len() as u64);
                let header = framing::header(&prefix, &Sha256::digest(&prefix));
                let mut checksum = Sha256::new();
                checksum.update(&header);
                checksum.update(&raw);
                let trailer = framing::trailer(Kind::Catalog, &checksum.finalize());
                c.catalog.log.invalidate();
                c.catalog.file.write_all(&header)?;
                c.catalog.file.write_all(&raw)?;
                c.catalog.file.write_all(&trailer)?;
                Err::<(), _>(std::io::Error::from_raw_os_error(5).into())
            })
            .unwrap_err();
        assert_eq!(error.kind, "StorageUnavailable");
        Ok(())
    }

    fn fail_refresh(catalog: &mut Catalog, config: &Config) {
        let error = catalog
            .refresh_with_sync(config, |_| Err(std::io::Error::from_raw_os_error(5).into()))
            .unwrap_err();
        assert_eq!(error.kind, "StorageUnavailable");
        assert!(catalog.log.dirty());
    }

    #[test]
    fn lost_consumption_frame_rebuilds_graph_and_owners_together() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        let path = config.root.join("storage.log");
        let stable = fs::read(&path)?;
        let node = format!("staged:{}", "a".repeat(64));
        let receipt = "b".repeat(64);
        uncertain(
            &mut catalog,
            &config,
            json!({"staging_edges":{&node:[]},"owners":{"source":null},"consumed_handoffs":[&receipt]}),
        )?;
        fail_refresh(&mut catalog, &config);
        assert!(catalog.log.indexes().staging_edges.contains_key(&node));
        assert!(catalog.handoff_consumed(&receipt));
        assert!(!catalog.log.indexes().owners.contains_key("source"));
        // Explicit storage-crash fixture: only the unconfirmed frame is lost.
        fs::write(&path, stable)?;
        catalog.transaction(&config, |c| {
            assert!(c.catalog.log.indexes().staging_edges.is_empty());
            assert!(!c.handoff_consumed(&receipt));
            assert!(c.catalog.log.indexes().owners.contains_key("source"));
            Ok(())
        })
    }

    #[test]
    fn buffered_replay_discards_every_torn_suffix_before_append() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        let path = config.root.join("storage.log");
        catalog.transaction(&config, |c| {
            c.catalog.append(
                &config,
                json!({"padding":"x".repeat(65536 + 17)}),
                &mut |_| Ok(()),
            )?;
            for i in 0..16 {
                c.catalog
                    .append(&config, json!({"ignored":i}), &mut |_| Ok(()))?;
            }
            Ok(())
        })?;
        let stable = fs::read(&path)?;
        let sequence = catalog.log.sequence();
        uncertain(
            &mut catalog,
            &config,
            json!({"owners":{"tail":["raw/pack"]}}),
        )?;
        let complete = fs::read(&path)?;
        let tail = &complete[stable.len()..];
        for cut in 0..=tail.len() {
            fs::write(&path, &complete[..stable.len() + cut])?;
            let mut reader = Catalog::open(&config)?;
            let valid = if cut == tail.len() {
                complete.len()
            } else {
                stable.len()
            };
            reader.transaction(&config, |c| {
                assert_eq!(c.catalog.log.offset(), valid as u64, "cut={cut}");
                assert_eq!(
                    c.catalog.file.stream_position()?,
                    c.catalog.log.offset(),
                    "cut={cut}"
                );
                assert_eq!(
                    c.catalog.log.indexes().owners.contains_key("tail"),
                    cut == tail.len()
                );
                c.catalog.append(
                    &config,
                    json!({"owners":{"after":["raw/pack"]}}),
                    &mut |_| Ok(()),
                )
            })?;
            assert_eq!(&fs::read(&path)?[..valid], &complete[..valid]);
            let mut reopened = Catalog::open(&config)?;
            reopened.transaction(&config, |c| {
                assert_eq!(
                    c.catalog.log.sequence(),
                    sequence + 1 + u64::from(cut == tail.len())
                );
                assert_eq!(
                    c.catalog.log.indexes().owners.len(),
                    2 + usize::from(cut == tail.len())
                );
                c.require_retained(&HashSet::from(["source".into(), "after".into()]))
            })?;
        }
        Ok(())
    }

    #[test]
    fn buffered_replay_rejects_corruption_across_refills_without_truncating() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        let path = config.root.join("storage.log");
        let prefix = catalog.log.offset() as usize;
        catalog.transaction(&config, |c| {
            c.catalog.append(
                &config,
                json!({"padding":"x".repeat(65536 + 17)}),
                &mut |_| Ok(()),
            )
        })?;
        let complete = fs::read(&path)?;
        for index in [prefix + 24, prefix + 56, 65535, 65536, complete.len() - 1] {
            let mut damaged = complete.clone();
            damaged[index] ^= 1;
            fs::write(&path, &damaged)?;
            let mut reader = Catalog::open(&config)?;
            assert_eq!(
                reader.transaction(&config, |_| Ok(())).unwrap_err().kind,
                "CorruptData"
            );
            assert_eq!(fs::read(&path)?, damaged);
        }
        Ok(())
    }

    // These tests provide an already acknowledged retirement token to isolate
    // the actual locked catalog half of queue GC from the queue WAL producer.
    fn prepared_roots<'a>(prefix: &'a str, keys: &[&str]) -> crate::queue_gc::Prepared<'a> {
        let roots = crate::queue_roots::Roots {
            references: keys
                .iter()
                .map(|key| (key.to_string(), Value::Null))
                .collect(),
            prefix: 0,
            positions: HashSet::new(),
        };
        crate::queue_gc::prepare(prefix, crate::queue_retirement::Retired::fixture(roots))
    }

    #[test]
    fn public_api_cannot_replace_release_or_reactivate_internal_queue_owners() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        let owner = "queue:control:root".to_owned();
        catalog.transaction(&config, |c| {
            c.catalog.append(
                &config,
                json!({"owners":{&owner:["raw/pack"]}}),
                &mut |_| Ok(()),
            )
        })?;
        let before = fs::read(config.root.join("storage.log"))?;
        catalog.transaction(&config, |c| {
            let mut effects = Vec::new();
            let mut fault = |at: &str| {
                effects.push(at.to_owned());
                Ok(())
            };
            assert_eq!(
                c.retain(&config, &owner, &[], &mut fault).unwrap_err().kind,
                "ValueError"
            );
            assert_eq!(
                c.retain_many(&config, &[(owner.clone(), vec![])], &mut fault)
                    .unwrap_err()
                    .kind,
                "ValueError"
            );
            assert_eq!(
                c.release(&config, std::slice::from_ref(&owner), &mut fault)
                    .unwrap_err()
                    .kind,
                "ValueError"
            );
            let roots = vec![(owner.clone(), Value::Null)];
            assert_eq!(
                c.retain_reactivated(&config, &roots, &mut fault)
                    .unwrap_err()
                    .kind,
                "ValueError"
            );
            let verified = VerifiedReactivation {
                root: config.root.clone(),
                run_id: config.run_id.clone(),
                roots,
                verified: HashSet::new(),
            };
            assert_eq!(
                c.retain_verified(&config, &verified, &mut fault)
                    .unwrap_err()
                    .kind,
                "ValueError"
            );
            assert_eq!(
                c.retain_queue_many(
                    &config,
                    "queue:other:",
                    &[(owner.clone(), vec![])],
                    &mut fault
                )
                .unwrap_err()
                .kind,
                "ValueError"
            );
            assert!(effects.is_empty());
            assert_eq!(
                c.catalog.log.indexes().owners[&owner],
                BTreeSet::from(["raw/pack".into()])
            );
            Ok(())
        })?;
        assert_eq!(fs::read(config.root.join("storage.log"))?, before);
        Ok(())
    }

    #[test]
    fn queue_collection_missing_root_stops_before_prune_or_io() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        catalog.transaction(&config, |c| {
            c.catalog.append(
                &config,
                json!({"owners":{"queue:a:old":["raw/pack"]}}),
                &mut |_| Ok(()),
            )
        })?;
        let before = fs::read(config.root.join("storage.log"))?;
        let mut effects = Vec::new();
        let error = catalog
            .transaction(&config, |c| {
                c.collect_queue(
                    &config,
                    prepared_roots("queue:a:", &["missing"]),
                    &mut |at| {
                        effects.push(at.to_owned());
                        Ok(())
                    },
                )
            })
            .unwrap_err();
        assert_eq!(error.kind, "UnsafeRecovery");
        assert!(effects.is_empty());
        assert_eq!(fs::read(config.root.join("storage.log"))?, before);
        assert!(catalog.log.indexes().owners.contains_key("queue:a:old"));
        Ok(())
    }

    #[test]
    fn queue_collection_prunes_only_obsolete_owners_and_retains_peers() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        fs::create_dir_all(config.root.join("raw"))?;
        for name in ["live", "old", "peer", "publication"] {
            fs::write(config.root.join("raw").join(name), name)?;
        }
        catalog.transaction(&config, |c| {
            c.catalog.append(
                &config,
                json!({
                    "owners":{"source":null,"queue:a:live":["raw/live"],"queue:a:old":["raw/old"],
                        "queue:b:peer":["raw/peer"],"publication:writer:1":["raw/publication"]},
                    "packs":{"raw/live":true,"raw/old":true,"raw/peer":true,"raw/publication":true}
                }),
                &mut |_| Ok(()),
            )
        })?;
        catalog.transaction(&config, |c| {
            c.collect_queue(&config, prepared_roots("queue:a:", &["live"]), &mut |_| {
                Ok(())
            })
        })?;
        assert!(!config.root.join("raw/old").exists());
        for name in ["live", "peer", "publication"] {
            assert!(config.root.join("raw").join(name).exists());
        }
        let mut reopened = Catalog::open(&config)?;
        reopened.transaction(&config, |c| {
            assert!(!c.catalog.log.indexes().owners.contains_key("queue:a:old"));
            for owner in ["queue:a:live", "queue:b:peer", "publication:writer:1"] {
                assert!(c.catalog.log.indexes().owners.contains_key(owner));
            }
            assert!(c.catalog.log.indexes().pending_deletions.is_empty());
            Ok(())
        })
    }

    #[test]
    fn queue_collection_rejects_reserved_owner_prefix_before_pruning() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        catalog.transaction(&config, |c| {
            c.catalog.append(
                &config,
                json!({"owners":{"publication:writer:1":["raw/pack"]}}),
                &mut |_| Ok(()),
            )
        })?;
        let before = fs::read(config.root.join("storage.log"))?;
        let error = catalog
            .transaction(&config, |c| {
                c.collect_queue(
                    &config,
                    prepared_roots("publication:", &[]),
                    &mut |_| Ok(()),
                )
            })
            .unwrap_err();
        assert_eq!(error.kind, "ValueError");
        assert_eq!(fs::read(config.root.join("storage.log"))?, before);
        assert!(
            catalog
                .log
                .indexes()
                .owners
                .contains_key("publication:writer:1")
        );
        Ok(())
    }

    #[test]
    fn collection_keeps_independent_mark_and_receipt_write_hooks() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        fs::create_dir_all(config.root.join("raw"))?;
        let pack = config.root.join("raw/pack");
        fs::write(&pack, b"receipt requires its own admission")?;
        catalog.transaction(&config, |c| {
            c.catalog.append(
                &config,
                json!({"owners":{"source":null},"packs":{"raw/pack":true}}),
                &mut |_| Ok(()),
            )
        })?;
        let mut writes = 0;
        let error = catalog
            .transaction(&config, |c| {
                c.collect(&config, &mut |at| {
                    if at == "before_storage_catalog_write" {
                        writes += 1;
                        if writes == 2 {
                            return Err(std::io::Error::from_raw_os_error(5).into());
                        }
                    }
                    Ok(())
                })
            })
            .unwrap_err();
        assert_eq!(error.kind, "StorageUnavailable");
        assert_eq!(writes, 2);
        assert!(!pack.exists());
        // The mark succeeded, but the independent receipt append did not start.
        let mut reopened = Catalog::open(&config)?;
        reopened.transaction(&config, |c| {
            assert!(
                c.catalog
                    .log
                    .indexes()
                    .pending_deletions
                    .contains("raw/pack")
            );
            let result = c.collect(&config, &mut |_| Ok(()))?;
            assert_eq!(result["reclaimed_files"], 0);
            assert!(c.catalog.log.indexes().pending_deletions.is_empty());
            assert!(c.catalog.log.indexes().deleted.contains("raw/pack"));
            Ok(())
        })
    }

    #[test]
    fn collection_repairs_missing_parent_before_receipt_and_retries_failed_barrier() -> Result<()> {
        let (_dir, config, mut catalog) = setup(true)?;
        let parent = config.root.join("raw");
        assert!(!parent.exists());
        let mut trace = Vec::new();
        let error = catalog
            .transaction(&config, |c| {
                c.collect(&config, &mut |at| {
                    trace.push(at.to_owned());
                    if at == "before_directory_sync" {
                        return Err(std::io::Error::from_raw_os_error(5).into());
                    }
                    Ok(())
                })
            })
            .unwrap_err();
        assert_eq!(error.kind, "StorageUnavailable");
        assert!(trace.iter().any(|at| at == "before_gc_parent_repair"));
        assert!(
            !trace
                .iter()
                .any(|at| at == "after_gc_parent_repair" || at == "before_gc_receipt")
        );
        assert!(parent.exists());
        // Simulate loss of the unconfirmed directory creation. Recovery must
        // enumerate and synchronize its full fresh chain before the receipt.
        fs::remove_dir(&parent)?;
        let mut reopened = Catalog::open(&config)?;
        let mut barriers = 0;
        reopened.transaction(&config, |c| {
            assert!(
                c.catalog
                    .log
                    .indexes()
                    .pending_deletions
                    .contains("raw/pack")
            );
            let result = c.collect(&config, &mut |at| {
                if at == "before_directory_sync" {
                    barriers += 1;
                }
                Ok(())
            })?;
            assert_eq!(result["reclaimed_files"], 0);
            assert!(c.catalog.log.indexes().pending_deletions.is_empty());
            Ok(())
        })?;
        assert_eq!(barriers, parent.canonicalize()?.ancestors().count());
        Ok(())
    }

    #[test]
    fn collection_binds_shared_parent_barrier_to_complete_receipt_targets() -> Result<()> {
        let (_dir, config, mut catalog) = setup(true)?;
        fs::create_dir_all(config.root.join("raw"))?;
        fs::write(config.root.join("raw/pack"), b"one")?;
        fs::write(config.root.join("raw/second"), b"two")?;
        catalog.transaction(&config, |c| {
            c.catalog.append(
                &config,
                json!({"packs":{"raw/second":true},"deleted":["raw/second"]}),
                &mut |_| Ok(()),
            )
        })?;
        let mut trace = Vec::new();
        let result = catalog.transaction(&config, |c| {
            c.collect(&config, &mut |at| {
                trace.push(at.to_owned());
                Ok(())
            })
        })?;
        assert_eq!(result["reclaimed_files"], 2);
        assert_eq!(result["reclaimed_bytes"], 6);
        let relevant = trace
            .iter()
            .map(String::as_str)
            .filter(|at| {
                matches!(
                    *at,
                    "before_gc_unlink" | "before_gc_directory_sync" | "before_gc_receipt"
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            relevant,
            vec![
                "before_gc_unlink",
                "before_gc_unlink",
                "before_gc_directory_sync",
                "before_gc_receipt"
            ]
        );
        let mut reopened = Catalog::open(&config)?;
        reopened.transaction(&config, |c| {
            for path in ["raw/pack", "raw/second"] {
                assert!(c.catalog.log.indexes().deleted.contains(path));
                assert!(!c.catalog.log.indexes().packs.contains_key(path));
            }
            assert!(c.catalog.log.indexes().pending_deletions.is_empty());
            Ok(())
        })
    }

    #[test]
    fn stale_stat_cannot_retire_an_existing_pack() -> Result<()> {
        for exists in [false, true] {
            let (_dir, config, mut catalog) = setup(true)?;
            fs::create_dir_all(config.root.join("raw"))?;
            let pack = config.root.join("raw/pack");
            if exists {
                fs::write(&pack, b"still present")?;
            }
            let mut unlinks = 0;
            let result = catalog.transaction(&config, |c| {
                c.catalog.collect_with_stat(
                    &config,
                    &mut |at| {
                        if at == "before_gc_unlink" {
                            unlinks += 1;
                        }
                        Ok(())
                    },
                    |_| Err(std::io::ErrorKind::NotFound.into()),
                )
            })?;
            assert_eq!(unlinks, 1);
            assert_eq!(result["reclaimed_files"], u64::from(exists));
            assert_eq!(result["reclaimed_bytes"], 0);
            assert!(!pack.exists());
            let mut replayed = Catalog::open(&config)?;
            replayed.transaction(&config, |c| {
                assert!(c.catalog.log.indexes().deleted.contains("raw/pack"));
                assert!(c.catalog.log.indexes().pending_deletions.is_empty());
                Ok(())
            })?;
        }
        Ok(())
    }

    #[test]
    fn stale_stat_does_not_hide_unlink_or_barrier_failure() -> Result<()> {
        for failure in ["before_gc_unlink", "before_gc_directory_sync"] {
            let (_dir, config, mut catalog) = setup(true)?;
            fs::create_dir_all(config.root.join("raw"))?;
            let pack = config.root.join("raw/pack");
            fs::write(&pack, b"retry me")?;
            let before = fs::read(config.root.join("storage.log"))?;
            let error = catalog
                .transaction(&config, |c| {
                    c.catalog.collect_with_stat(
                        &config,
                        &mut |at| {
                            if at == failure {
                                return Err(std::io::Error::from_raw_os_error(5).into());
                            }
                            Ok(())
                        },
                        |_| Err(std::io::ErrorKind::NotFound.into()),
                    )
                })
                .unwrap_err();
            assert_eq!(error.kind, "StorageUnavailable");
            assert_eq!(pack.exists(), failure == "before_gc_unlink");
            assert_eq!(fs::read(config.root.join("storage.log"))?, before);
            let mut replayed = Catalog::open(&config)?;
            replayed.transaction(&config, |c| {
                assert!(
                    c.catalog
                        .log
                        .indexes()
                        .pending_deletions
                        .contains("raw/pack")
                );
                c.collect(&config, &mut |_| Ok(()))?;
                assert!(c.catalog.log.indexes().pending_deletions.is_empty());
                Ok(())
            })?;
            assert!(!pack.exists());
        }
        Ok(())
    }

    #[test]
    fn empty_failed_barrier_keeps_cache_but_observes_new_peer_state() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        let floor = catalog.log.floor();
        let error = catalog
            .refresh_with_sync(
                &config,
                |_| Err(std::io::Error::from_raw_os_error(5).into()),
            )
            .unwrap_err();
        assert_eq!(error.kind, "StorageUnavailable");
        assert!(!catalog.log.dirty());
        assert_eq!(catalog.log.floor(), floor);
        assert!(catalog.log.indexes().owners.contains_key("source"));
        assert_eq!(
            catalog
                .require_retained(&HashSet::from(["source".into()]))
                .unwrap_err()
                .kind,
            "UnsafeRecovery"
        );
        let mut peer = Catalog::open(&config)?;
        peer.transaction(&config, |c| {
            c.catalog.append(
                &config,
                json!({"owners":{"source":null},"packs":{"raw/pack":true}}),
                &mut |_| Ok(()),
            )
        })?;
        catalog.transaction(&config, |c| {
            assert_eq!(
                c.require_retained(&HashSet::from(["source".into()]))
                    .unwrap_err()
                    .kind,
                "UnsafeRecovery"
            );
            assert_eq!(
                c.pack(&config, "raw/pack", false, &mut |_| Ok(()))
                    .unwrap_err()
                    .kind,
                "InvalidReference"
            );
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn empty_failed_barrier_still_rejects_confirmed_prefix_loss() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        let path = config.root.join("storage.log");
        let floor = catalog.log.floor();
        let error = catalog
            .refresh_with_sync(
                &config,
                |_| Err(std::io::Error::from_raw_os_error(5).into()),
            )
            .unwrap_err();
        assert_eq!(error.kind, "StorageUnavailable");
        assert!(!catalog.log.dirty());
        fs::OpenOptions::new()
            .write(true)
            .open(&path)?
            .set_len(floor - 1)?;
        let damaged = fs::read(&path)?;
        assert_eq!(
            catalog.transaction(&config, |_| Ok(())).unwrap_err().kind,
            "UnsafeRecovery"
        );
        assert_eq!(fs::read(path)?, damaged);
        Ok(())
    }

    #[test]
    fn partial_apply_is_marked_before_its_error() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        let path = config.root.join("storage.log");
        let stable = fs::read(&path)?;
        inject_invalid_unconfirmed_frame(
            &mut catalog,
            &config,
            json!({
                "owners":{"tentative":["raw/pack"]}, "reclaimed":["raw/not-tombstoned"]
            }),
        )?;
        let bad_tail = fs::read(&path)?;
        assert_eq!(
            catalog.transaction(&config, |_| Ok(())).unwrap_err().kind,
            "CorruptData"
        );
        assert!(catalog.log.indexes().owners.contains_key("tentative"));
        assert!(catalog.log.dirty());
        assert_eq!(
            fs::read(&path)?,
            bad_tail,
            "Corrupt frames are not silently repaired"
        );
        // Explicitly simulate removal of a failed unconfirmed malformed tail.
        fs::write(&path, &stable)?;
        catalog.transaction(&config, |c| {
            c.require_retained(&HashSet::from(["source".into()]))?;
            assert_eq!(
                c.require_retained(&HashSet::from(["tentative".into()]))
                    .unwrap_err()
                    .kind,
                "UnsafeRecovery"
            );
            assert_eq!(c.catalog.log.indexes().owners.len(), 1);
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn lost_unconfirmed_tail_rebuilds_all_catalog_indexes() -> Result<()> {
        for (receipt, event) in [
            (false, json!({"owners":{"tentative":["raw/pack"]}})),
            (false, json!({"owners":{"source":null}})),
            (false, json!({"packs":{"raw/pack":true}})),
            (
                false,
                json!({"owners":{"source":null},"packs":{"raw/pack":true},"deleted":["raw/pack"]}),
            ),
            (true, json!({"reclaimed":["raw/pack"]})),
        ] {
            let (_dir, config, mut catalog) = setup(receipt)?;
            let path = config.root.join("storage.log");
            let stable = fs::read(&path)?;
            let owners = catalog.log.indexes().owners.clone();
            let packs = catalog.log.indexes().packs.clone();
            let deleted = catalog.log.indexes().deleted.clone();
            let pending = catalog.log.indexes().pending_deletions.clone();
            let sequence = catalog.log.sequence();
            uncertain(&mut catalog, &config, event)?;
            fail_refresh(&mut catalog, &config);
            assert!(catalog.log.offset() > stable.len() as u64);
            assert_eq!(catalog.log.floor(), stable.len() as u64);
            catalog
                .closures
                .insert("tentative-cache".into(), BTreeSet::new());
            fs::write(&path, &stable)?;
            catalog.transaction(&config, |c| {
                assert_eq!(c.catalog.log.indexes().owners, owners);
                assert_eq!(c.catalog.log.indexes().packs, packs);
                assert_eq!(c.catalog.log.indexes().deleted, deleted);
                assert_eq!(c.catalog.log.indexes().pending_deletions, pending);
                assert!(c.closures.is_empty());
                assert_eq!(c.catalog.log.sequence(), sequence);
                assert_eq!(c.catalog.log.offset(), stable.len() as u64);
                assert!(!c.catalog.log.dirty());
                Ok(())
            })?;
            assert_eq!(fs::read(path)?, stable);
        }
        Ok(())
    }

    #[test]
    fn surviving_tail_replays_after_repeated_failed_barriers() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        let floor = catalog.log.floor();
        uncertain(
            &mut catalog,
            &config,
            json!({"owners":{"tentative":["raw/pack"]}}),
        )?;
        let complete = fs::read(config.root.join("storage.log"))?;
        for _ in 0..3 {
            fail_refresh(&mut catalog, &config);
            assert_eq!(catalog.log.floor(), floor);
        }
        catalog.transaction(&config, |c| {
            assert_eq!(c.catalog.log.indexes().owners.len(), 2);
            assert_eq!(c.catalog.log.floor(), complete.len() as u64);
            assert!(!c.catalog.log.dirty());
            Ok(())
        })?;
        assert_eq!(fs::read(config.root.join("storage.log"))?, complete);
        Ok(())
    }

    #[test]
    fn rebuild_still_rejects_loss_of_a_confirmed_prefix() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        let path = config.root.join("storage.log");
        let floor = catalog.log.floor();
        uncertain(
            &mut catalog,
            &config,
            json!({"owners":{"tentative":["raw/pack"]}}),
        )?;
        fail_refresh(&mut catalog, &config);
        fs::OpenOptions::new()
            .write(true)
            .open(&path)?
            .set_len(floor - 1)?;
        let damaged = fs::read(&path)?;
        let error = catalog.transaction(&config, |_| Ok(())).unwrap_err();
        assert_eq!(error.kind, "UnsafeRecovery");
        assert_eq!(fs::read(path)?, damaged);
        assert_eq!(catalog.log.floor(), floor);
        Ok(())
    }

    #[test]
    fn rebuild_cannot_truncate_an_incomplete_confirmed_prefix() -> Result<()> {
        let (_dir, config, mut catalog) = setup(false)?;
        let path = config.root.join("storage.log");
        uncertain(
            &mut catalog,
            &config,
            json!({"owners":{"tentative":["raw/pack"]}}),
        )?;
        fail_refresh(&mut catalog, &config);
        let mut damaged = fs::read(&path)?;
        let too_long = damaged.len() as u64 + 1;
        damaged[16..24].copy_from_slice(&too_long.to_le_bytes());
        let checksum = Sha256::digest(&damaged[..24]);
        damaged[24..56].copy_from_slice(&checksum);
        fs::write(&path, &damaged)?;
        let error = catalog.transaction(&config, |_| Ok(())).unwrap_err();
        assert_eq!(error.kind, "UnsafeRecovery");
        assert!(error.message.contains("prefix is incomplete"));
        assert_eq!(fs::read(path)?, damaged);
        Ok(())
    }
}
#[path = "gc_writer.rs"]
mod writer_retirement;
pub use writer_retirement::RetirementPolicy;
