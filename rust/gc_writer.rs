//! Retirement of registered writer incarnations under their permanent gate.
use super::{Catalog, LockedCatalog};
use crate::store::{Config, Reader};
use crate::writer_gate::{Operation, Permit, Registry};
use crate::{Error, Fault, Result, fail};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Reserved owners can only be changed through the internal writer and queue
/// protocols. External durable roots must have a distinct owner identity.
pub(super) fn public_owners<'a>(owners: impl IntoIterator<Item = &'a str>) -> Result<()> {
    if owners
        .into_iter()
        .any(|owner| !crate::queue_gc::public_owner(owner))
    {
        return fail(
            "ValueError",
            "publication: and queue: are reserved for internal durable operations",
        );
    }
    Ok(())
}

pub(super) fn queue_owners<'a>(
    prefix: &str,
    owners: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    if !prefix.starts_with("queue:")
        || prefix.len() <= "queue:".len()
        || !prefix.ends_with(':')
        || owners
            .into_iter()
            .any(|owner| !owner.starts_with(prefix) || owner.len() <= prefix.len())
    {
        return fail(
            "ValueError",
            "Queue owner does not belong to its control namespace",
        );
    }
    Ok(())
}

/// Recover every completely described result, or explicitly abandon unresolved
/// operations. Both policies preserve existing staging, queue and reader roots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetirementPolicy {
    Recover,
    Abandon,
}

impl LockedCatalog<'_> {
    pub(crate) fn release_preparation(
        &mut self,
        config: &Config,
        owners: &[String],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.catalog.release_preparation(config, owners, fault)
    }
    fn check_writer_owners(
        &mut self,
        config: &Config,
        writer: &str,
        operations: &[Operation],
    ) -> Result<()> {
        self.catalog.require_ready()?;
        let prefix = format!("publication:{writer}:");
        let owners = self
            .catalog
            .log
            .indexes()
            .owners
            .iter()
            .filter(|(owner, _)| {
                crate::writer_ownership::starts_with(owner.as_bytes(), prefix.as_bytes())
            })
            .map(|(owner, paths)| crate::writer_ownership::Held {
                owner: owner.clone(),
                paths: paths.iter().cloned().collect(),
            })
            .collect::<Vec<_>>();
        let mut coverage = OwnerCoverage {
            catalog: self.catalog,
            config,
            paths: BTreeSet::new(),
        };
        match crate::writer_ownership::validate_owners(
            &mut coverage,
            operations,
            &owners,
            prefix.as_bytes(),
        ) {
            Ok(()) => {}
            Err(crate::writer_ownership::Failure::Unregistered) => {
                return fail(
                    "UnsafeRecovery",
                    "Writer has an unregistered preparation owner; retain it and resolve its origin before retirement",
                );
            }
            Err(crate::writer_ownership::Failure::Uncertified) => {
                return fail(
                    "UnsafeRecovery",
                    "Preparation owner has paths not certified by its registered dependency evidence",
                );
            }
        }
        Ok(())
    }
    fn seal_writer(&mut self, config: &Config, writer: &str, fault: &mut Fault<'_>) -> Result<()> {
        self.catalog.require_context(config)?;
        let directory = config
            .path(&format!("raw/{}/{writer}", &writer[..2]))?
            .canonicalize()?;
        let paths = self
            .catalog
            .log
            .indexes()
            .packs
            .iter()
            .filter(|(_, sealed)| !**sealed)
            .map(|(path, sealed)| (path.clone(), *sealed))
            .collect::<Vec<_>>();
        let mut resolver = WriterParents { config };
        let selected = crate::writer_ownership::seal_candidates(&mut resolver, &paths, &directory)?;
        let mut packs = serde_json::Map::new();
        for index in selected {
            packs.insert(paths[index].0.clone(), json!(true));
        }
        if packs.is_empty() {
            return Ok(());
        }
        self.catalog.append(config, json!({"packs":packs}), fault)
    }
}
struct OwnerCoverage<'a> {
    catalog: &'a mut Catalog,
    config: &'a Config,
    paths: BTreeSet<String>,
}
impl crate::writer_ownership::CoverageModel for OwnerCoverage<'_> {}
impl crate::writer_ownership::Coverage for OwnerCoverage<'_> {
    fn reset(&mut self) {
        self.paths.clear();
    }
    fn include(&mut self, dependency: &Value) {
        // A failed closure contributes no positive evidence. The checked
        // caller chooses only dependencies belonging to the exact held owner.
        if let Ok(closure) = self.catalog.closure(self.config, dependency) {
            self.paths.extend(closure);
        }
    }
    fn contains(&self, path: &str) -> bool {
        self.paths.contains(path)
    }
}
struct WriterParents<'a> {
    config: &'a Config,
}
impl crate::writer_ownership::ParentModel for WriterParents<'_> {
    type Identity = std::path::PathBuf;
}
impl crate::writer_ownership::Parents for WriterParents<'_> {
    type Error = Error;
    fn canonical_parent(&mut self, path: &str) -> Result<Option<std::path::PathBuf>> {
        let full = self.config.path(path)?;
        let parent = full
            .parent()
            .ok_or_else(|| Error::new("InvalidReference", "Pack has no writer directory"))?;
        match parent.canonicalize() {
            Ok(parent) => Ok(Some(parent)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    fn same_parent(&self, left: &std::path::PathBuf, right: &std::path::PathBuf) -> bool {
        left == right
    }
}

impl Catalog {
    fn release_preparation(
        &mut self,
        config: &Config,
        owners: &[String],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        self.require_context(config)?;
        if owners
            .iter()
            .any(|owner| !owner.starts_with("publication:"))
        {
            return fail("ValueError", "Not a preparation owner");
        }
        let mut resolver = &mut self.namespace;
        let mut writer =
            crate::wal_file::Appender::new(&mut self.file, crate::framing::Kind::Catalog, fault);
        crate::queue_gc::release(
            &mut self.log,
            &mut resolver,
            &config.run_id,
            owners,
            &mut writer,
        )
        .map_err(super::catalog_append_error)
    }
}

struct Retirement<'a, 'f> {
    config: &'a Config,
    writer: &'a str,
    permit: Permit,
    registry: Registry,
    policy: RetirementPolicy,
    fault: &'a mut Fault<'f>,
    recovered: Vec<Value>,
    pending: Vec<Operation>,
}
impl crate::writer_retirement::Effects for Retirement<'_, '_> {
    type Error = Error;
    fn fence(&mut self) -> Result<()> {
        (self.fault)("before_writer_fence")?;
        self.permit
            .durable_marker(crate::writer_gate_flow::Marker::Fence, self.fault)?;
        (self.fault)("after_writer_fence")
    }
    fn drain(&mut self) -> Result<()> {
        // The lock excludes live users and the filesystem must finish/cancel
        // already submitted operations before handing the lock to a new holder.
        // These fsync barriers additionally persist files and directory cleanup.
        self.permit.drain_files(self.fault)?;
        (self.fault)("after_writer_drain")
    }
    fn reconcile(&mut self) -> Result<()> {
        self.config
            .catalog(|c| c.check_writer_owners(self.config, self.writer, &self.pending))?;
        let mut io = Recovery {
            config: self.config,
            writer: self.writer,
            registry: &mut self.registry,
            fault: self.fault,
        };
        crate::writer_registry::reconcile_registered(
            &mut io,
            &self.pending,
            self.policy == RetirementPolicy::Recover,
            &mut self.recovered,
        )?;
        (self.fault)("after_writer_reconcile")
    }
    fn seal(&mut self) -> Result<()> {
        self.config
            .catalog(|c| c.seal_writer(self.config, self.writer, self.fault))?;
        (self.fault)("after_writer_seal")
    }
    fn record(&mut self) -> Result<()> {
        self.permit
            .durable_marker(crate::writer_gate_flow::Marker::Retired, self.fault)?;
        (self.fault)("after_writer_retirement_receipt")
    }
}
// Every expansion authenticates exactly one node. The shared checked closure
// traversal visits its complete record/dependency graph; keys label nodes here.
struct AuthenticatedReader {
    reader: Reader,
}
impl crate::closure::SourceModel for AuthenticatedReader {
    type Reference = Value;
}
impl crate::closure::Source for AuthenticatedReader {
    type Error = Error;
    fn expand(
        &mut self,
        key: &String,
        reference: &Value,
    ) -> Result<crate::closure::Expansion<Value>> {
        let children = match crate::closure_node::kind(reference) {
            crate::closure_node::Kind::Manifest => {
                let (records, dependencies) = self.reader.validate_node(reference, None, None)?;
                if !crate::writer_ownership::result_dependencies(&dependencies) {
                    return fail(
                        "InvalidReference",
                        "Result dependency must be a record-set reference",
                    );
                }
                crate::closure_node::children(
                    &mut super::ReferenceDigest,
                    &json!({"records":records,"dependencies":dependencies}),
                )
                .map_err(super::closure_node_error)?
            }
            crate::closure_node::Kind::Record => {
                self.reader.verify(reference)?;
                Vec::new()
            }
            crate::closure_node::Kind::Invalid => {
                return fail(
                    "InvalidReference",
                    "Expected a record or record-set reference",
                );
            }
        };
        Ok(crate::closure::Expansion {
            pack: key.clone(),
            children,
        })
    }
}

struct Recovery<'a, 'f> {
    config: &'a Config,
    writer: &'a str,
    registry: &'a mut Registry,
    fault: &'a mut Fault<'f>,
}
impl crate::writer_registry::RecoveryIo for Recovery<'_, '_> {
    type Error = Error;
    fn validate(&mut self, roots: &[Value]) -> Result<()> {
        let mut source = AuthenticatedReader {
            reader: Reader::new(self.config.clone()),
        };
        crate::writer_ownership::authenticate_all(&mut source, &mut super::ReferenceDigest, roots)
            .map_err(|error| match error {
                crate::writer_ownership::AuthenticationFailure::Read(error)
                | crate::writer_ownership::AuthenticationFailure::Digest(error) => error,
            })?;
        Ok(())
    }
    fn stage(&mut self, roots: &[Value]) -> Result<()> {
        self.config.stage(roots, self.fault)
    }
    fn consume(&mut self, key: &str, refs: &[Value]) -> Result<()> {
        let handoff =
            crate::writer_gate::intermediate_handoff(&self.config.run_id, self.writer, key, refs)?;
        self.config
            .catalog(|c| c.consume_once(self.config, &handoff, refs, self.fault))?;
        Ok(())
    }
    fn release(&mut self, owner: &str) -> Result<()> {
        self.config
            .catalog(|c| c.release_preparation(self.config, &[owner.to_owned()], self.fault))?;
        Ok(())
    }
    fn commit(&mut self, ids: &[u64], roots: &[Value]) -> Result<()> {
        self.registry.retired(ids, roots, self.fault)
    }
    fn checkpoint(&mut self) -> Result<()> {
        (self.fault)("after_writer_operation_reconcile")
    }
}

impl Config {
    /// Read the latest recorded result for one exact writer/API/request.
    /// Settlement does not prove that a caller received its reply. This query
    /// also works after retirement and never renews staging or other ownership.
    /// Old references may already have been explicitly consumed and collected.
    pub fn publication_receipt(
        &self,
        writer: &str,
        api: &str,
        request: &str,
        fault: &mut Fault<'_>,
    ) -> Result<Option<Vec<Value>>> {
        let api = match api {
            "records" => crate::finalization::Api::Records,
            "record_set" => crate::finalization::Api::RecordSet,
            "publish" => crate::finalization::Api::Publish,
            _ => {
                return fail(
                    "ValueError",
                    "Expected records, record_set or publish receipt API",
                );
            }
        };
        // Acquiring the existing gate is also valid for fenced incarnations.
        // It does not itself fence, recover payloads or change owner state.
        let mut permit = Permit::retire(&self.root, &self.run_id, writer)?;
        permit.receipt(
            &crate::writer_gate::operation_key(writer, api, request),
            fault,
        )
    }
    /// Retire one exact incarnation using its native persistent gate. A paused
    /// holder returns WriterBusy; it is never classified as dead by a timeout.
    /// Success permanently fences further Store writes, reconciles registered
    /// publications, seals its reservations and persists a retirement receipt.
    /// Legacy/unregistered owners are retained and reported as UnsafeRecovery.
    /// Returned recovered results remain staged until explicitly consumed.
    pub fn retire_writer(
        &self,
        writer: &str,
        policy: RetirementPolicy,
        fault: &mut Fault<'_>,
    ) -> Result<Vec<Value>> {
        let mut permit = Permit::retire(&self.root, &self.run_id, writer)?;
        let registry = permit.registry(fault)?;
        let recovered = registry.retired_results();
        let pending = registry.pending();
        let mut retirement = Retirement {
            config: self,
            writer,
            permit,
            registry,
            policy,
            fault,
            recovered,
            pending,
        };
        crate::writer_retirement::execute(&mut retirement)?;
        Ok(retirement.recovered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Record, Store};
    fn config(root: &std::path::Path, target: u64) -> Config {
        let mut config = Config::new(
            root.to_str().unwrap(),
            "retirement",
            vec!["bytes.v1".into()],
            1 << 20,
            1 << 22,
            target,
        )
        .unwrap();
        config.online_gc = true;
        config
    }
    fn record() -> Record {
        Record {
            metadata: json!({"record_id":"r","codec":"bytes.v1","metadata":{},"tokens":0}),
            payload: b"persistent data".to_vec().into(),
        }
    }
    #[test]
    fn retired_writer_cannot_resume_and_live_staging_survives() -> Result<()> {
        let root = tempfile::tempdir()?;
        let config = config(root.path(), 1 << 20);
        let mut writer = Store::new(config.clone());
        let refs = writer.write(&[record()], "first", &mut |_| Ok(()))?;
        let id = writer.writer_id().to_owned();
        assert!(
            config
                .retire_writer(&id, RetirementPolicy::Recover, &mut |_| Ok(()))?
                .is_empty()
        );
        assert_eq!(
            writer
                .write(&[record()], "next", &mut |_| Ok(()))
                .unwrap_err()
                .kind,
            "WriterFenced"
        );
        assert_eq!(config.read(&refs[0])?.payload.as_ref(), b"persistent data");
        let before = config
            .catalog(|c| c.collect(&config, &mut |_| Ok(())))?
            .unwrap();
        assert_eq!(before["open_packs"], 0);
        assert_eq!(before["reclaimed_files"], 0);
        config.catalog(|c| c.consume(&config, &refs, &mut |_| Ok(())))?;
        let after = config
            .catalog(|c| c.collect(&config, &mut |_| Ok(())))?
            .unwrap();
        assert_eq!(after["reclaimed_files"], 1);
        assert!(
            config
                .retire_writer(&id, RetirementPolicy::Recover, &mut |_| Ok(()))?
                .is_empty()
        );
        Ok(())
    }
    #[test]
    fn durable_done_receipt_survives_lost_reply_and_never_restages() -> Result<()> {
        let root = tempfile::tempdir()?;
        let config = config(root.path(), 1 << 20);
        let mut writer = Store::new(config.clone());
        let id = writer.writer_id().to_owned();
        let failure = writer.write(&[record()], "lost", &mut |at| {
            if at == "after_publication_receipt" {
                return fail(
                    "StorageUnavailable",
                    "caller did not receive completed reply",
                );
            }
            Ok(())
        });
        assert_eq!(failure.unwrap_err().kind, "StorageUnavailable");
        drop(writer);
        let query = || config.publication_receipt(&id, "records", "lost", &mut |_| Ok(()));
        let receipt = query()?.unwrap();
        assert_eq!(receipt.len(), 1);
        assert_eq!(
            config.read(&receipt[0])?.payload.as_ref(),
            b"persistent data"
        );
        assert!(
            config
                .publication_receipt(&id, "publish", "lost", &mut |_| Ok(()))?
                .is_none()
        );
        assert!(
            config
                .publication_receipt(&id, "records", "missing", &mut |_| Ok(()))?
                .is_none()
        );
        assert!(
            config
                .retire_writer(&id, RetirementPolicy::Recover, &mut |_| Ok(()))?
                .is_empty()
        );
        assert_eq!(query()?.unwrap(), receipt);
        config.catalog(|c| c.consume(&config, &receipt, &mut |_| Ok(())))?;
        assert_eq!(
            config
                .catalog(|c| c.collect(&config, &mut |_| Ok(())))?
                .unwrap()["reclaimed_files"],
            1
        );
        assert_eq!(query()?.unwrap(), receipt);
        assert!(config.read(&receipt[0]).is_err());
        assert_eq!(
            config
                .catalog(|c| c.collect(&config, &mut |_| Ok(())))?
                .unwrap()["owners"],
            0
        );
        Ok(())
    }
    #[test]
    fn retirement_retries_every_persistent_effect_and_returns_recovered_results() -> Result<()> {
        for at in [
            "after_writer_fence",
            "after_writer_drain",
            "after_writer_operation_reconcile",
            "after_writer_reconcile",
            "after_writer_seal",
            "after_writer_retirement_receipt",
        ] {
            let root = tempfile::tempdir()?;
            let config = config(root.path(), 1 << 20);
            let mut writer = Store::new(config.clone());
            // Results are registered before staging. Fail the stage append;
            // recovery must authenticate/stage the saved descriptors itself.
            let mut seen = 0;
            let failed = writer.write(&[record()], "uncertain", &mut |point| {
                if point == "before_storage_catalog_sync" {
                    seen += 1;
                    if seen == 2 {
                        return fail("StorageUnavailable", "lost staging reply");
                    }
                }
                Ok(())
            });
            assert!(failed.is_err());
            assert_eq!(seen, 2);
            let id = writer.writer_id().to_owned();
            drop(writer);
            let mut injected = false;
            assert!(
                config
                    .retire_writer(&id, RetirementPolicy::Recover, &mut |point| {
                        if point == at && !injected {
                            injected = true;
                            return fail("StorageUnavailable", "retirement reply lost");
                        }
                        Ok(())
                    })
                    .is_err(),
                "{at}"
            );
            assert!(injected, "{at}");
            let results = config.retire_writer(&id, RetirementPolicy::Recover, &mut |_| Ok(()))?;
            assert_eq!(results.len(), 1, "{at}");
            assert_eq!(
                config.read(&results[0])?.payload.as_ref(),
                b"persistent data"
            );
            assert_eq!(
                results,
                config.retire_writer(&id, RetirementPolicy::Recover, &mut |_| Ok(()))?
            );
        }
        Ok(())
    }
    #[test]
    fn held_gate_is_busy_and_fence_survives_reopening() -> Result<()> {
        let root = tempfile::tempdir()?;
        let config = config(root.path(), 1 << 20);
        let mut writer = Store::new(config.clone());
        writer.write(&[record()], "first", &mut |_| Ok(()))?;
        let id = writer.writer_id().to_owned();
        let permit = Permit::retire(&config.root, &config.run_id, &id)?;
        assert_eq!(
            config
                .retire_writer(&id, RetirementPolicy::Recover, &mut |_| Ok(()))
                .unwrap_err()
                .kind,
            "WriterBusy"
        );
        drop(permit);
        config.retire_writer(&id, RetirementPolicy::Recover, &mut |_| Ok(()))?;
        assert_eq!(
            writer.seal(&mut |_| Ok(())).unwrap_err().kind,
            "WriterFenced"
        );
        Ok(())
    }
    #[test]
    fn unknown_preparation_owner_is_retained_and_public_collision_is_rejected() -> Result<()> {
        let root = tempfile::tempdir()?;
        let config = config(root.path(), 1 << 20);
        let mut writer = Store::new(config.clone());
        let refs = writer.write(&[record()], "first", &mut |_| Ok(()))?;
        let owner = format!("publication:{}:legacy", writer.writer_id());
        assert_eq!(
            config
                .catalog(|c| c.retain(&config, &owner, &refs, &mut |_| Ok(())))
                .unwrap_err()
                .kind,
            "ValueError"
        );
        config.catalog(|c| {
            c.catalog.append(
                &config,
                json!({"owners":{&owner:[refs[0]["segment"]["path"].clone()]}}),
                &mut |_| Ok(()),
            )
        })?;
        assert_eq!(
            config
                .retire_writer(writer.writer_id(), RetirementPolicy::Abandon, &mut |_| Ok(
                    ()
                ))
                .unwrap_err()
                .kind,
            "UnsafeRecovery"
        );
        config.catalog(|c| {
            assert!(c.catalog.log.indexes().owners.contains_key(&owner));
            assert!(c.catalog.log.indexes().packs.values().any(|sealed| !sealed));
            Ok(())
        })?;
        Ok(())
    }
    #[test]
    fn legacy_and_foreign_identity_are_never_falsely_fenced() -> Result<()> {
        let root = tempfile::tempdir()?;
        let config = config(root.path(), 1 << 20);
        assert_eq!(
            config
                .retire_writer(
                    "00000000000000000000000000000000",
                    RetirementPolicy::Abandon,
                    &mut |_| Ok(())
                )
                .unwrap_err()
                .kind,
            "UnsafeRecovery"
        );
        let mut writer = Store::new(config.clone());
        writer.write(&[record()], "first", &mut |_| Ok(()))?;
        let mut foreign = config.clone();
        foreign.run_id = "foreign".into();
        assert_eq!(
            foreign
                .retire_writer(writer.writer_id(), RetirementPolicy::Recover, &mut |_| Ok(
                    ()
                ))
                .unwrap_err()
                .kind,
            "UnsafeRecovery"
        );
        writer.write(&[record()], "second", &mut |_| Ok(()))?;
        Ok(())
    }

    #[test]
    fn recovery_transfers_exact_dependency_evidence_before_releasing_preparation() -> Result<()> {
        let root = tempfile::tempdir()?;
        let config = config(root.path(), 1 << 20);
        let mut source = Store::new(config.clone());
        let refs = source.write(&[record()], "source", &mut |_| Ok(()))?;
        source.close()?;
        let mut writer = Store::new(config.clone());
        let mut seen = 0;
        assert!(
            writer
                .record_set(&refs, &[], "result", &mut |point| {
                    if point == "before_storage_catalog_sync" {
                        seen += 1;
                        if seen == 3 {
                            return fail("StorageUnavailable", "lost first result staging reply");
                        }
                    }
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(seen, 3);
        config.catalog(|c| c.consume(&config, &refs, &mut |_| Ok(())))?;
        let mut permit = Permit::retire(&config.root, &config.run_id, writer.writer_id())?;
        let pending = permit.registry(&mut |_| Ok(()))?.pending();
        let intermediates = pending
            .iter()
            .find_map(|op| op.results.as_ref())
            .unwrap()
            .intermediates
            .clone();
        drop(permit);
        let mut restaged = false;
        assert!(
            config
                .retire_writer(
                    writer.writer_id(),
                    RetirementPolicy::Recover,
                    &mut |point| {
                        if point == "after_writer_operation_reconcile" && !restaged {
                            restaged = true;
                            config.stage(&intermediates, &mut |_| Ok(()))?;
                            return fail(
                                "StorageUnavailable",
                                "consumption reply lost before a new staging generation",
                            );
                        }
                        Ok(())
                    }
                )
                .is_err()
        );
        assert!(restaged);
        let results =
            config.retire_writer(writer.writer_id(), RetirementPolicy::Recover, &mut |_| {
                Ok(())
            })?;
        assert_eq!(results.len(), 1);
        config.catalog(|c| {
            for reference in &intermediates {
                assert!(
                    c.catalog
                        .log
                        .indexes()
                        .owners
                        .contains_key(&Catalog::staged(reference)?)
                );
            }
            Ok(())
        })?;
        assert_eq!(
            config
                .catalog(|c| c.collect(&config, &mut |_| Ok(())))?
                .unwrap()["reclaimed_files"],
            0
        );
        let mut reader = Reader::new(config.clone());
        let recovered = reader.validate(&results[0], None, None)?;
        assert_eq!(recovered.len(), 1);
        assert_eq!(
            reader.read(&recovered[0])?.payload.as_ref(),
            b"persistent data"
        );
        config.catalog(|c| c.consume(&config, &results, &mut |_| Ok(())))?;
        config.catalog(|c| c.consume(&config, &intermediates, &mut |_| Ok(())))?;
        assert_eq!(
            config
                .catalog(|c| c.collect(&config, &mut |_| Ok(())))?
                .unwrap()["reclaimed_files"],
            2
        );
        Ok(())
    }
}
