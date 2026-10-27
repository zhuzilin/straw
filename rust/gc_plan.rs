//! Durable deletion plans survive releasing the catalog lock. Permanent
//! tombstones and never-reused pack names allow collectors to overlap safely.
use super::{LockedCatalog, PhysicalIo, catalog_append_error, queue_preparation_error};
use crate::gc_collection::Physical;
use crate::store::Config;
use crate::{Error, Fault, Result, fail};
use std::fs::File;
use std::path::{Path, PathBuf};

pub(crate) struct Plan {
    root: PathBuf,
    run: String,
    log: File,
    targets: Vec<String>,
    retained: usize,
}

// Only completion of all unlinks and directory barriers constructs this token.
struct Completed {
    plan: Plan,
    statistics: crate::gc_batch::Statistics,
}

impl LockedCatalog<'_> {
    pub(crate) fn plan_collection(
        &mut self,
        config: &Config,
        prepared: Option<crate::queue_gc::Prepared<'_>>,
        fault: &mut Fault<'_>,
    ) -> Result<Plan> {
        self.catalog.require_context(config)?;
        let catalog = &mut *self.catalog;
        let mut writer =
            crate::wal_file::Appender::new(&mut catalog.file, crate::framing::Kind::Catalog, fault);
        if let Some(prepared) = prepared {
            crate::gc_collection::prepare_queue(
                &mut catalog.log,
                &mut catalog.namespace,
                &config.run_id,
                &mut writer,
                prepared,
            )
            .map_err(queue_preparation_error)?;
            writer.restart();
        }
        let marked = crate::gc_mark::mark(
            &mut catalog.log,
            &mut catalog.namespace,
            &config.run_id,
            &mut writer,
        )
        .map_err(catalog_append_error)?;
        writer.finish();
        Ok(Plan {
            root: config.root.clone(),
            run: config.run_id.clone(),
            log: catalog.file.try_clone()?,
            targets: marked.targets,
            retained: marked.live.len(),
        })
    }

    fn complete_collection(
        &mut self,
        config: &Config,
        completed: Completed,
        fault: &mut Fault<'_>,
    ) -> Result<serde_json::Value> {
        self.catalog.require_context(config)?;
        let Completed { plan, statistics } = completed;
        if plan.root != config.root
            || plan.run != config.run_id
            || crate::catalog_primitives::identity(&plan.log)? != self.catalog.log_identity
            || plan
                .targets
                .iter()
                .any(|p| !self.catalog.log.indexes().deleted.contains(p))
        {
            return fail(
                "UnsafeRecovery",
                "Deletion receipt belongs to another catalog or unmarked path",
            );
        }
        // Another collector may have already recorded some or all receipts.
        // Never retire a new pending target merely because it appeared meanwhile.
        let pending = plan
            .targets
            .into_iter()
            .filter(|p| self.catalog.log.indexes().pending_deletions.contains(p))
            .collect::<Vec<_>>();
        if !pending.is_empty() {
            fault("before_gc_receipt")?;
            let catalog = &mut *self.catalog;
            let mut writer = crate::wal_file::Appender::new(
                &mut catalog.file,
                crate::framing::Kind::Catalog,
                fault,
            );
            crate::gc_receipt::record(
                &mut catalog.log,
                &mut catalog.namespace,
                &config.run_id,
                &pending,
                &mut writer,
            )
            .map_err(catalog_append_error)?;
            writer.finish();
        }
        Ok(self
            .catalog
            .collection_summary(crate::gc_collection::Collected {
                statistics,
                retained: plan.retained,
            }))
    }
}

impl Plan {
    pub(crate) fn execute(
        self,
        config: &Config,
        fault: &mut Fault<'_>,
    ) -> Result<serde_json::Value> {
        if self.root != config.root || self.run != config.run_id {
            return fail(
                "UnsafeRecovery",
                "Deletion plan belongs to another storage pool",
            );
        }
        // Both the in-process mutex and filesystem catalog lock are released.
        fault("after_gc_tombstone")?;
        let mut physical = Deletion {
            io: PhysicalIo {
                config,
                stat: |path| std::fs::metadata(path),
            },
            targets: &self.targets,
            fault,
        };
        let statistics = crate::gc_batch::delete(&mut physical).map_err(|error| match error {
            crate::gc_batch::Failure::Effect(error) => error,
            crate::gc_batch::Failure::Protocol => {
                Error::new("UnsafeRecovery", "Deletion batch did not finish")
            }
            crate::gc_batch::Failure::Accounting => {
                Error::new("CorruptData", "Deletion accounting exceeds u64")
            }
        })?;
        let completed = Completed {
            plan: self,
            statistics,
        };
        config
            .catalog(|c| c.complete_collection(config, completed, fault))?
            .ok_or_else(|| Error::new("UnsafeRecovery", "Deletion lost its catalog"))
    }
}

type Stat = fn(&Path) -> std::io::Result<std::fs::Metadata>;
struct Deletion<'a, 'f> {
    io: PhysicalIo<'a, Stat>,
    targets: &'a [String],
    fault: &'a mut Fault<'f>,
}
impl crate::gc_batch::BatchModel for Deletion<'_, '_> {
    type Parent = PathBuf;
}
impl crate::gc_io::Model for Deletion<'_, '_> {
    type Path = PathBuf;
    type Directory = File;
}
impl crate::gc_io::Primitives for Deletion<'_, '_> {
    type Error = Error;
    fn resolve(&self, index: usize) -> Result<PathBuf> {
        self.io.resolve(&self.targets[index])
    }
    fn parent(&self, index: usize, path: &PathBuf) -> PathBuf {
        self.io.parent(&self.targets[index], path)
    }
    fn point(&mut self, point: &str) -> Result<()> {
        (self.fault)(point)
    }
    fn stat(&mut self, _index: usize, path: &PathBuf) -> Result<Option<u64>> {
        self.io.stat(path)
    }
    fn remove(&mut self, index: usize, path: &PathBuf) -> Result<bool> {
        self.io.remove(&self.targets[index], path)
    }
    fn open_parent(&mut self, parent: &PathBuf) -> Result<Option<File>> {
        self.io.open_parent(parent)
    }
    fn sync(&mut self, directory: &File) -> Result<()> {
        self.io.sync_parent(directory)
    }
    fn repair(&mut self, parent: &PathBuf) -> Result<()> {
        // This path is intentionally not cached: absence may follow an uncertain
        // deletion and every required ancestor must be confirmed on this attempt.
        crate::store::mkdir(parent, self.fault)
    }
}
impl crate::gc_batch::Effects for Deletion<'_, '_> {
    type Error = Error;
    fn target_count(&self) -> usize {
        self.targets.len()
    }
    fn marked(&self, _index: usize) -> bool {
        true
    } // Private, durably admitted Plan.
    fn unlink(&mut self, index: usize) -> Result<crate::gc_batch::Removal<PathBuf>> {
        crate::gc_io::unlink(self, index)
    }
    fn sync_parent(&mut self, parent: &PathBuf) -> Result<()> {
        crate::gc_io::sync_parent(self, parent)
    }
    fn receipt(&mut self) -> Result<()> {
        unreachable!("Detached deletion cannot write a receipt")
    }
}
