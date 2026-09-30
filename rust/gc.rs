//! Cross-queue ownership catalog. One append log/lock inode per storage pool.
//!
//! Writers register open packs before creating them. Publications and queue
//! owners retain transitive pack paths before releasing their predecessor.
//! Unknown/crashed writers and uncertain publications leak safely; no TTLs.
use crate::store::{Config, Reader};
use crate::{Error, Fault, Result, array, bytes, digest, fail, field};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

pub struct Catalog {
    file: File,
    offset: u64,
    sequence: u64,
    owners: HashMap<String, BTreeSet<String>>,
    packs: HashMap<String, bool>,
    deleted: HashSet<String>,
    closures: HashMap<String, BTreeSet<String>>,
}

impl Catalog {
    pub fn open(config: &Config) -> Result<Self> {
        crate::store::mkdir(&config.root, &mut |_| Ok(()))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(config.root.join("storage.log"))?;
        File::open(&config.root)?.sync_all()?;
        Ok(Self {
            file,
            offset: 0,
            sequence: 0,
            owners: HashMap::new(),
            packs: HashMap::new(),
            deleted: HashSet::new(),
            closures: HashMap::new(),
        })
    }

    pub fn transaction<T>(
        &mut self,
        config: &Config,
        call: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        // A distributed advisory lock serializes owners; it does not by itself
        // establish freshness of an already open remote filesystem handle.
        // Keep the lock's open description alive while reopening for I/O.
        let lock = self.file.try_clone()?;
        lock.lock()?;
        let result = (|| {
            self.file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(config.root.join("storage.log"))?;
            self.refresh(config)?;
            call(self)
        })();
        let unlocked = lock.unlock();
        match result {
            Ok(v) => {
                unlocked?;
                Ok(v)
            }
            Err(e) => Err(e),
        }
    }

    fn refresh(&mut self, config: &Config) -> Result<()> {
        let size = self.file.metadata()?.len();
        if size < self.offset {
            return fail("UnsafeRecovery", "Storage catalog was truncated");
        }
        self.file.seek(SeekFrom::Start(self.offset))?;
        while self.offset < size {
            let remaining = size - self.offset;
            if remaining < 56 {
                let mut partial = vec![0; remaining as usize];
                self.file.read_exact(&mut partial)?;
                if !b"STRGC001".starts_with(&partial[..partial.len().min(8)]) {
                    return fail("CorruptData", "Corrupt partial storage catalog header");
                }
                break;
            }
            let mut header = [0u8; 56];
            self.file.read_exact(&mut header)?;
            if &header[..8] != b"STRGC001"
                || Sha256::digest(&header[..24]).as_slice() != &header[24..]
            {
                return fail(
                    "CorruptData",
                    "Storage catalog header checksum/magic mismatch",
                );
            }
            let sequence = u64::from_le_bytes(header[8..16].try_into().unwrap());
            let n = u64::from_le_bytes(header[16..24].try_into().unwrap());
            if sequence != self.sequence {
                return fail(
                    "CorruptData",
                    "Storage catalog length/sequence outside bounds",
                );
            }
            let frame_size = n.checked_add(96).ok_or_else(|| {
                Error::new("CorruptData", "Storage catalog frame length overflow")
            })?;
            if remaining < frame_size {
                break;
            }
            let mut raw = vec![0; n as usize];
            self.file.read_exact(&mut raw)?;
            let mut trailer = [0u8; 40];
            self.file.read_exact(&mut trailer)?;
            let mut checksum = Sha256::new();
            checksum.update(header);
            checksum.update(&raw);
            if &trailer[32..] != b"STRGEND1" || checksum.finalize().as_slice() != &trailer[..32] {
                return fail("CorruptData", "Storage catalog checksum/trailer mismatch");
            }
            let event: Value = serde_json::from_slice(&raw)?;
            if event["run_id"] != config.run_id {
                return fail("UnsafeRecovery", "Storage pool run identity mismatch");
            }
            self.apply(&event)?;
            self.offset += 96 + n;
            self.sequence += 1;
        }
        // Complete frames surviving a failed sync must become durable before
        // allowing another owner to drop the protection that they established.
        if self.offset != size {
            self.file.set_len(self.offset)?;
        }
        self.file.sync_all()?;
        self.file.seek(SeekFrom::Start(self.offset))?;
        Ok(())
    }

    fn apply(&mut self, event: &Value) -> Result<()> {
        for (owner, paths) in event["owners"]
            .as_object()
            .into_iter()
            .flat_map(|v| v.iter())
        {
            if paths.is_null() {
                self.owners.remove(owner);
            } else {
                self.owners
                    .insert(owner.clone(), serde_json::from_value(paths.clone())?);
            }
        }
        for (path, sealed) in event["packs"]
            .as_object()
            .into_iter()
            .flat_map(|v| v.iter())
        {
            self.packs.insert(
                path.clone(),
                sealed
                    .as_bool()
                    .ok_or_else(|| Error::new("CorruptData", "Invalid pack state"))?,
            );
        }
        for path in event["deleted"].as_array().into_iter().flatten() {
            self.deleted.insert(
                path.as_str()
                    .ok_or_else(|| Error::new("CorruptData", "Invalid tombstone"))?
                    .into(),
            );
        }
        Ok(())
    }

    fn append(&mut self, config: &Config, mut event: Value, fault: &mut Fault<'_>) -> Result<()> {
        event["run_id"] = json!(config.run_id);
        let raw = bytes(&event)?;
        let mut header = b"STRGC001".to_vec();
        header.extend_from_slice(&self.sequence.to_le_bytes());
        header.extend_from_slice(&(raw.len() as u64).to_le_bytes());
        header.extend_from_slice(&Sha256::digest(header.clone()));
        let mut checksum = Sha256::new();
        checksum.update(&header);
        checksum.update(&raw);
        let mut trailer = checksum.finalize().to_vec();
        trailer.extend_from_slice(b"STRGEND1");
        fault("before_storage_catalog_write")?;
        for part in [&header, &raw, &trailer] {
            self.file.write_all(part)?;
            fault("after_storage_catalog_part")?;
        }
        fault("before_storage_catalog_sync")?;
        self.file.sync_all()?;
        fault("after_storage_catalog_sync")?;
        self.apply(&event)?;
        self.offset += 96 + raw.len() as u64;
        self.sequence += 1;
        Ok(())
    }

    pub fn pack(
        &mut self,
        config: &Config,
        path: &str,
        sealed: bool,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        if self.deleted.contains(path) {
            return fail("InvalidReference", "Pack was reclaimed");
        }
        if self.packs.get(path) == Some(&sealed) {
            return Ok(());
        }
        if self.packs.get(path) == Some(&true) {
            return fail("InvalidReference", "Cannot reopen a sealed pack");
        }
        self.append(config, json!({"packs":{path:sealed}}), fault)
    }

    pub fn closure(&mut self, config: &Config, reference: &Value) -> Result<BTreeSet<String>> {
        self.closure_inner(
            reference,
            &mut HashSet::new(),
            &mut Reader::new(config.clone()),
        )
    }

    fn closure_inner(
        &mut self,
        reference: &Value,
        inspected: &mut HashSet<String>,
        reader: &mut Reader,
    ) -> Result<BTreeSet<String>> {
        let key = digest(reference)?;
        let paths = if let Some(paths) = self.closures.get(&key) {
            paths.clone()
        } else {
            let mut paths = BTreeSet::new();
            if reference.get("manifest").is_some() {
                let contents = reader.manifest(reference)?;
                paths.insert(field(&reference["manifest"]["segment"], "path")?.into());
                for member in array(&contents, "records")? {
                    paths.extend(self.closure_inner(member, inspected, reader)?);
                }
                for dep in array(&contents, "dependencies")? {
                    paths.extend(self.closure_inner(dep, inspected, reader)?);
                }
            } else if reference.get("segment").is_some() {
                // Every member in one immutable extent shares its index.
                // Re-reading that full index per ordinal makes retaining an
                // N-record publication quadratic. Limit reuse to this retain
                // operation and key by the complete descriptor, not its path.
                let segment = digest(&reference["segment"])?;
                if !inspected.contains(&segment) {
                    reader.index(&reference["segment"])?;
                    inspected.insert(segment);
                }
                paths.insert(field(&reference["segment"], "path")?.into());
            } else {
                return fail(
                    "InvalidReference",
                    "Expected a record or record-set reference",
                );
            }
            if self.closures.len() > 100000 {
                self.closures.clear();
            }
            self.closures.insert(key, paths.clone());
            paths
        };
        if paths.iter().any(|p| self.deleted.contains(p)) {
            return fail(
                "InvalidReference",
                "Reference has been reclaimed; retain before releasing its source owner",
            );
        }
        Ok(paths)
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
    pub fn retain_many(
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
                paths.extend(self.closure_inner(r, &mut inspected, &mut reader)?);
            }
            if paths.is_empty() && !self.owners.contains_key(owner) {
                continue;
            }
            if self.owners.get(owner) != Some(&paths) {
                updates.insert(owner.clone(), json!(paths));
            }
        }
        if updates.is_empty() {
            return Ok(());
        }
        self.append(config, json!({"owners":updates}), fault)
    }

    /// Explicit consumer restore may adopt roots retained by a checkpoint or
    /// another owner. GC never repairs ownership on its own. All validation
    /// happens under the catalog lock, before either catalog or queue WAL write.
    pub fn retain_reactivated(
        &mut self,
        config: &Config,
        roots: &[(String, Value)],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        let missing = roots
            .iter()
            .filter(|(owner, _)| !self.owners.contains_key(owner))
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(());
        }
        let protected = self
            .owners
            .values()
            .flatten()
            .cloned()
            .collect::<HashSet<_>>();
        let mut updates = serde_json::Map::new();
        let mut reader = Reader::new(config.clone());
        let mut inspected = HashSet::new();
        for (owner, reference) in missing {
            let paths = self.closure_inner(reference, &mut inspected, &mut reader)?;
            if paths.is_empty() || paths.iter().any(|p| !protected.contains(p)) {
                return fail(
                    "UnsafeRecovery",
                    "Restored queue root has no durable source owner",
                );
            }
            reader.validate(reference, None, None)?;
            // A rewind is rare. Check every reachable payload, including a
            // previously cached closure, before re-authorizing queue reads.
            let mut pending = vec![reference.clone()];
            let mut checked = HashSet::new();
            while let Some(reference) = pending.pop() {
                if !checked.insert(digest(&reference)?) {
                    continue;
                }
                let manifest = reader.manifest(&reference)?;
                for record in array(&manifest, "records")? {
                    reader.read(record)?;
                }
                pending.extend(array(&manifest, "dependencies")?.iter().cloned());
            }
            updates.insert(owner.clone(), json!(paths));
        }
        self.append(config, json!({"owners":updates}), fault)
    }

    pub fn release(
        &mut self,
        config: &Config,
        owners: &[String],
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        let updates: serde_json::Map<_, _> = owners
            .iter()
            .filter(|o| self.owners.contains_key(*o))
            .map(|o| (o.clone(), Value::Null))
            .collect();
        if updates.is_empty() {
            return Ok(());
        }
        self.append(config, json!({"owners":updates}), fault)
    }

    pub fn require_retained(&self, owners: &HashSet<String>) -> Result<()> {
        for owner in owners {
            let Some(paths) = self.owners.get(owner) else {
                return fail(
                    "UnsafeRecovery",
                    "Live queue root has no durable storage owner",
                );
            };
            if paths.is_empty() || paths.iter().any(|p| self.deleted.contains(p)) {
                return fail(
                    "UnsafeRecovery",
                    "Live queue root has invalid storage ownership",
                );
            }
        }
        Ok(())
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
        let mut reader = Reader::new(config.clone());
        let mut todo = refs.to_vec();
        let mut owners = HashSet::new();
        while let Some(r) = todo.pop() {
            if !owners.insert(Self::staged(&r)?) {
                continue;
            }
            if r.get("manifest").is_some() {
                let m = reader.manifest(&r)?;
                todo.extend(array(&m, "dependencies")?.iter().cloned());
                todo.extend(array(&m, "records")?.iter().cloned());
            }
        }
        self.release(config, &owners.into_iter().collect::<Vec<_>>(), fault)
    }

    pub fn prune(
        &mut self,
        config: &Config,
        prefix: &str,
        live: &HashSet<String>,
        fault: &mut Fault<'_>,
    ) -> Result<()> {
        let dead = self
            .owners
            .keys()
            .filter(|o| o.starts_with(prefix) && !live.contains(*o))
            .cloned()
            .collect::<Vec<_>>();
        self.release(config, &dead, fault)
    }

    pub fn collect(&mut self, config: &Config, fault: &mut Fault<'_>) -> Result<Value> {
        let live: HashSet<_> = self.owners.values().flatten().cloned().collect();
        let candidates = self
            .packs
            .iter()
            .filter(|(p, sealed)| **sealed && !live.contains(*p) && !self.deleted.contains(*p))
            .map(|(p, _)| p.clone())
            .collect::<Vec<_>>();
        // A durable tombstone fences later retain attempts before any unlink.
        if !candidates.is_empty() {
            self.append(config, json!({"deleted":candidates}), fault)?;
        }
        fault("after_gc_tombstone")?;
        let mut files = 0u64;
        let mut reclaimed = 0u64;
        for path in &self.deleted {
            let full = config.path(path)?;
            match std::fs::metadata(&full) {
                Ok(m) => {
                    fault("before_gc_unlink")?;
                    // Another client may have unlinked this durable tombstone
                    // while our attribute cache still reports the old inode.
                    // Only a successful unlink counts as newly reclaimed data.
                    let removed = match std::fs::remove_file(&full) {
                        Ok(()) => true,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                        Err(e) => return Err(e.into()),
                    };
                    File::open(full.parent().unwrap())?.sync_all()?;
                    if removed {
                        files += 1;
                        reclaimed += m.len();
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(
            json!({"reclaimed_files":files,"reclaimed_bytes":reclaimed,"owners":self.owners.len(),
            "open_packs":self.packs.iter().filter(|(p,s)| !**s && !self.deleted.contains(*p)).count(),
            "retained_packs":live.len(),"tombstones":self.deleted.len()}),
        )
    }
}
