use crate::store::{mkdir, sync, sync_dir};
use crate::{Error, Fault, Result, bytes, fail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};
const MAGIC: &[u8] = b"SLMTXN01";
const END: &[u8] = b"SLMTEND1";
pub struct Journal {
    pub path: PathBuf,
    file: Option<File>,
    pub sequence: u64,
    pub transactions: Vec<Value>,
    pub poisoned: bool,
    pub incomplete_tail: u64,
    read_only: bool,
}
impl Journal {
    pub fn open(
        root: &std::path::Path,
        identity: &Value,
        recover: bool,
        read_only: bool,
        fault: &mut Fault<'_>,
    ) -> Result<Self> {
        if read_only && !recover {
            return fail("ValueError", "Read-only journal requires recovery");
        }
        let path = root.join("control/journal.log");
        if !read_only {
            mkdir(path.parent().unwrap(), fault)?;
        }
        let mut options = OpenOptions::new();
        options.read(true);
        if !read_only {
            options.write(true);
            if !recover {
                options.create_new(true);
            }
        }
        let file = options.open(&path)?;
        let mut result = Self {
            path,
            file: Some(file),
            sequence: 0,
            transactions: vec![],
            poisoned: false,
            incomplete_tail: 0,
            read_only,
        };
        if recover {
            result.recover(fault)?;
            if result.transactions.first() != Some(&json!([{"type":"Run","identity":identity}])) {
                return fail(
                    "UnsafeRecovery",
                    "Journal run/configuration identity mismatch",
                );
            }
        } else {
            sync(result.file.as_ref().unwrap(), fault)?;
            sync_dir(result.path.parent().unwrap(), fault)?;
            result.append(&json!([{"type":"Run","identity":identity}]), fault)?;
        }
        Ok(result)
    }
    fn recover(&mut self, fault: &mut Fault<'_>) -> Result<()> {
        let f = self.file.as_mut().unwrap();
        let size = f.metadata()?.len();
        let mut valid = 0;
        while valid < size {
            f.seek(SeekFrom::Start(valid))?;
            let remaining = size - valid;
            if remaining < 56 {
                let mut partial = vec![0; remaining as usize];
                f.read_exact(&mut partial)?;
                if !MAGIC.starts_with(&partial[..partial.len().min(8)]) {
                    return fail("CorruptData", "Corrupt partial journal header");
                }
                break;
            }
            let mut header = [0u8; 56];
            f.read_exact(&mut header)?;
            if Sha256::digest(&header[..24]).as_slice() != &header[24..] {
                return fail("CorruptData", "Journal header checksum mismatch");
            }
            let sequence = u64::from_le_bytes(header[8..16].try_into().unwrap());
            let length = u64::from_le_bytes(header[16..24].try_into().unwrap());
            if &header[..8] != MAGIC || sequence != self.sequence {
                return fail("CorruptData", "Journal sequence, magic or length mismatch");
            }
            let frame_size = length
                .checked_add(96)
                .ok_or_else(|| Error::new("CorruptData", "Journal frame length overflow"))?;
            if remaining < frame_size {
                break;
            }
            let mut body = vec![0; (length + 40) as usize];
            f.read_exact(&mut body)?;
            let n = length as usize;
            let mut h = Sha256::new();
            h.update(header);
            h.update(&body[..n]);
            if &body[n + 32..] != END || h.finalize().as_slice() != &body[n..n + 32] {
                return fail("CorruptData", "Complete journal transaction is corrupt");
            }
            let events: Value = serde_json::from_slice(&body[..n])?;
            if events.as_array().is_none_or(|a| {
                a.is_empty() || a.iter().any(|e| !e.is_object() || !e["type"].is_string())
            }) {
                return fail("CorruptData", "Invalid journal event list");
            }
            self.transactions.push(events);
            self.sequence += 1;
            valid += 56 + length + 40;
        }
        self.incomplete_tail = size - valid;
        if !self.read_only {
            if valid != size {
                f.set_len(valid)?;
            }
            sync(f, fault)?;
        }
        f.seek(SeekFrom::Start(valid))?;
        Ok(())
    }
    pub fn append(&mut self, events: &Value, fault: &mut Fault<'_>) -> Result<u64> {
        if self.poisoned || self.file.is_none() || self.read_only {
            return fail(
                "UnsafeRecovery",
                "Journal unavailable; stop owner and recover",
            );
        }
        let payload = bytes(events)?;
        let mut prefix = MAGIC.to_vec();
        prefix.extend_from_slice(&self.sequence.to_le_bytes());
        prefix.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        let mut header = prefix.clone();
        header.extend_from_slice(&Sha256::digest(prefix));
        let mut h = Sha256::new();
        h.update(&header);
        h.update(&payload);
        let mut trailer = h.finalize().to_vec();
        trailer.extend_from_slice(END);
        let f = self.file.as_mut().unwrap();
        let write: Result<()> = (|| {
            fault("before_journal_write")?;
            for part in [&header, &payload, &trailer] {
                f.write_all(part)?;
                fault("after_journal_part")?;
            }
            fault("before_journal_sync")?;
            sync(f, fault)?;
            fault("after_journal_sync")?;
            Ok(())
        })();
        if let Err(e) = write {
            self.poisoned = true;
            return Err(
                if ["StorageUnavailable", "QuotaExceeded"].contains(&e.kind.as_str()) {
                    Error::new(
                        "IndeterminateCommit",
                        "Journal write/sync outcome unknown; recover and query original ID",
                    )
                } else {
                    e
                },
            );
        }
        self.sequence += 1;
        Ok(self.sequence - 1)
    }
    pub fn close(&mut self) {
        self.file = None;
    }
    pub fn closed(&self) -> bool {
        self.file.is_none()
    }
    pub fn size(&self) -> u64 {
        self.path.metadata().map(|m| m.len()).unwrap_or(0)
    }
}
