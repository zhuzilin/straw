//! Native byte-I/O adapter for the shared replay driver. The operating-system
//! effects and the business callback remain separate refinement boundaries.
#[cfg(test)]
use crate::wal_device::Device;
#[cfg(test)]
use crate::wal_device::Target;
use crate::wal_recovery::{Failure, Malformed};
use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::fs::File;
#[cfg(test)]
use std::io::{Seek, SeekFrom};

// These thin native adapters expose File/BufReader operations; the replay
// ownership, state transitions and uncertainty guard live in wal_source.
pub struct NativeLibrary;
pub struct NativeDigest(Sha256);
impl crate::file_adapter::Digest for NativeDigest {
    fn new() -> Self {
        Self(Sha256::new())
    }
    fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
    fn finish(self) -> Vec<u8> {
        self.0.finalize().to_vec()
    }
}
impl crate::file_adapter::Library for NativeLibrary {
    type Error = Error;
    type Digest = NativeDigest;
    fn io(error: std::io::Error) -> Error {
        error.into()
    }
    fn unavailable(message: &str) -> Error {
        Error::new("UnsafeRecovery", message)
    }
}
pub type NativeFile<'a> = crate::file_adapter::NativeFile<'a, NativeLibrary>;
#[cfg(test)]
pub struct NativeBarrier<F>(pub(crate) F);

#[cfg(test)]
pub type Source<'a, F> = crate::wal_source::Source<NativeFile<'a>, NativeBarrier<F>>;

#[cfg(test)]
impl<'a, F: FnOnce(&File) -> Result<()>> Source<'a, F> {
    pub fn new(file: &'a mut File, sync: F) -> Result<Self> {
        Self::from_file(NativeFile::new(file), NativeBarrier(sync))
    }
}

#[cfg(test)]
impl<F: FnOnce(&File) -> Result<()>> crate::wal_source::Barrier<NativeFile<'_>>
    for NativeBarrier<F>
{
    fn sync(self, file: &mut NativeFile<'_>) -> Result<()> {
        (self.0)(file.file())
    }
}

#[cfg(test)]
pub struct Apply<F>(pub F);

#[cfg(test)]
impl<F: FnMut(&[u8]) -> Result<()>> Target for Apply<F> {
    type Error = Error;

    fn apply(&mut self, payload: &[u8]) -> Result<()> {
        self.0(payload)
    }
}

pub fn error(error: Failure<Error, Error>) -> Error {
    match error {
        Failure::Device(error) | Failure::Apply(error) => error,
        Failure::Malformed(kind) => match kind {
            Malformed::Cursor => Error::new(
                "UnsafeRecovery",
                "Log was truncated before the replay cursor",
            ),
            Malformed::ConfirmedPrefix => {
                Error::new("UnsafeRecovery", "Durable log prefix is incomplete")
            }
            Malformed::PartialHeader => Error::new("CorruptData", "Corrupt partial log header"),
            Malformed::Header => Error::new(
                "CorruptData",
                "Log header checksum, sequence or magic mismatch",
            ),
            Malformed::FrameLength => Error::new("CorruptData", "Log frame length outside bounds"),
            Malformed::Trailer => Error::new("CorruptData", "Complete log transaction is corrupt"),
            Malformed::Sequence => Error::new("CorruptData", "Log sequence overflow"),
        },
    }
}

pub struct NativeHooks<'a, 'f>(pub(crate) &'a mut crate::Fault<'f>);
pub type Appender<'a, 'f> = crate::wal_sink::Appender<NativeFile<'a>, NativeHooks<'a, 'f>>;

impl<'a, 'f> Appender<'a, 'f> {
    pub fn new(
        file: &'a mut File,
        kind: crate::framing::Kind,
        fault: &'a mut crate::Fault<'f>,
    ) -> Self {
        Self::from_file(NativeFile::new(file), kind, NativeHooks(fault))
    }
}

impl crate::wal_sink::Hooks<NativeFile<'_>> for NativeHooks<'_, '_> {
    fn point(&mut self, _file: &mut NativeFile<'_>, point: &str) -> Result<()> {
        (self.0)(point)
    }
}

pub fn append_error(error: crate::wal_append::Failure<Error>) -> Error {
    match error {
        crate::wal_append::Failure::Device(error) => error,
        crate::wal_append::Failure::Cursor => Error::new(
            "UnsafeRecovery",
            "Log append cursor does not match the confirmed boundary",
        ),
        crate::wal_append::Failure::Boundary => Error::new(
            "CorruptData",
            "Log append sequence or frame length outside bounds",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::{self, Kind};
    use std::fs::OpenOptions;
    use std::io::Write;

    #[test]
    fn shared_append_round_trips_both_log_kinds_and_empty_payloads() -> Result<()> {
        for kind in [Kind::Queue, Kind::Catalog] {
            let root = tempfile::tempdir()?;
            let path = root.path().join("journal");
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)?;
            let payloads = [Vec::new(), b"payload after empty frame".to_vec()];
            let mut offset = 0;
            let mut sequence = 0;
            for payload in &payloads {
                let mut no_fault = |_: &str| Ok(());
                let mut writer = Appender::new(&mut file, kind, &mut no_fault);
                let appended =
                    crate::wal_append::append(&mut writer, kind, sequence, offset, payload)
                        .map_err(append_error)?;
                sequence = appended.sequence;
                offset = appended.end;
            }
            assert_eq!(file.stream_position()?, offset);
            assert_eq!(file.metadata()?.len(), offset);
            let before = std::fs::read(&path)?;
            let mut source = Source::new(&mut file, |file: &File| {
                file.sync_all()?;
                Ok(())
            })?;
            let mut records = Vec::new();
            let mut target = Apply(|raw: &[u8]| {
                records.push(raw.to_vec());
                Ok(())
            });
            let boundary =
                crate::wal_recovery::recover(&mut source, &mut target, kind, 0, 0, 0, true)
                    .map_err(error)?;
            assert_eq!(boundary.sequence, sequence);
            assert_eq!(boundary.end, offset);
            assert_eq!(boundary.incomplete, 0);
            assert_eq!(records, payloads);
            assert_eq!(std::fs::read(&path)?, before);
        }
        Ok(())
    }

    #[test]
    fn append_rejects_mismatched_cursor_length_and_sequence_before_effects() -> Result<()> {
        for kind in [Kind::Queue, Kind::Catalog] {
            for invalid in ["cursor", "length", "sequence"] {
                let root = tempfile::tempdir()?;
                let path = root.path().join("journal");
                let mut file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                file.write_all(b"existing prefix")?;
                let before = std::fs::read(&path)?;
                let mut offset = before.len() as u64;
                let mut sequence = 1;
                match invalid {
                    "cursor" => {
                        file.seek(SeekFrom::Start(0))?;
                    }
                    "length" => {
                        offset += 1;
                        file.seek(SeekFrom::Start(offset))?;
                    }
                    _ => sequence = u64::MAX,
                }
                let mut points = Vec::new();
                let mut fault = |point: &str| {
                    points.push(point.to_string());
                    Ok(())
                };
                let mut writer = Appender::new(&mut file, kind, &mut fault);
                let rejected =
                    crate::wal_append::append(&mut writer, kind, sequence, offset, b"new data");
                if invalid == "sequence" {
                    assert!(matches!(
                        rejected,
                        Err(crate::wal_append::Failure::Boundary)
                    ));
                } else {
                    assert!(matches!(rejected, Err(crate::wal_append::Failure::Cursor)));
                }
                assert!(points.is_empty(), "invalid append reached write/sync hooks");
                assert_eq!(std::fs::read(&path)?, before);
            }
        }
        Ok(())
    }

    fn frame() -> Vec<u8> {
        let payload = b"replay payload";
        let prefix = framing::prefix(Kind::Queue, 0, payload.len() as u64);
        let header = framing::header(&prefix, &Sha256::digest(&prefix));
        let mut hash = Sha256::new();
        hash.update(&header);
        hash.update(payload);
        [
            header,
            payload.to_vec(),
            framing::trailer(Kind::Queue, &hash.finalize()),
        ]
        .concat()
    }

    #[test]
    fn replay_and_truncation_stay_bound_to_the_opened_log() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("journal");
        let moved = root.path().join("opened-journal");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        let bytes = frame();
        file.write_all(&bytes)?;
        file.write_all(b"S")?;
        let mut source = Source::new(&mut file, |file: &File| {
            file.sync_all()?;
            Ok(())
        })?;
        std::fs::rename(&path, &moved)?;
        std::fs::write(&path, b"replacement inode")?;
        let mut applied = Vec::new();
        let mut target = Apply(|raw: &[u8]| {
            applied.push(raw.to_vec());
            Ok(())
        });
        let boundary =
            crate::wal_recovery::recover(&mut source, &mut target, Kind::Queue, 0, 0, 0, true)
                .map_err(error)?;
        assert_eq!(boundary.end, bytes.len() as u64);
        assert_eq!(boundary.incomplete, 1);
        assert_eq!(source.length()?, boundary.end);
        drop(source);
        assert_eq!(file.stream_position()?, boundary.end);
        assert_eq!(applied, vec![b"replay payload".to_vec()]);
        assert_eq!(std::fs::read(&moved)?, bytes);
        assert_eq!(std::fs::read(&path)?, b"replacement inode");
        Ok(())
    }

    #[test]
    fn failed_barrier_after_truncation_requires_a_new_recovery_attempt() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("journal");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        let bytes = frame();
        file.write_all(&bytes)?;
        file.write_all(b"S")?;
        let mut failed = Source::new(&mut file, |_file: &File| {
            Err(Error::new("StorageUnavailable", "injected barrier failure"))
        })?;
        let mut count = 0;
        let mut target = Apply(|_raw: &[u8]| {
            count += 1;
            Ok(())
        });
        let result =
            crate::wal_recovery::recover(&mut failed, &mut target, Kind::Queue, 0, 0, 0, true);
        assert!(matches!(result, Err(Failure::Device(_))));
        assert_eq!(count, 1);
        assert_eq!(std::fs::read(&path)?, bytes);
        assert_eq!(failed.length().unwrap_err().kind, "UnsafeRecovery");
        assert_eq!(failed.seek(0).unwrap_err().kind, "UnsafeRecovery");
        drop(failed);
        let mut reopened = Source::new(&mut file, |file: &File| {
            file.sync_all()?;
            Ok(())
        })?;
        let mut target = Apply(|_raw: &[u8]| Ok(()));
        let boundary =
            crate::wal_recovery::recover(&mut reopened, &mut target, Kind::Queue, 0, 0, 0, true)
                .map_err(error)?;
        assert_eq!(boundary.sequence, 1);
        assert_eq!(boundary.incomplete, 0);
        drop(reopened);
        assert_eq!(file.stream_position()?, bytes.len() as u64);
        Ok(())
    }

    #[test]
    fn partial_read_failure_invalidates_the_cached_snapshot() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("journal");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.set_len(96)?;
        let external = OpenOptions::new().write(true).open(&path)?;
        let mut source = Source::new(&mut file, |_file: &File| Ok(()))?;
        source.seek(0)?;
        // Simulate storage rollback after the length snapshot was acquired.
        // An interrupted read must not make that old snapshot usable again.
        external.set_len(40)?;
        assert!(source.read(96).is_err());
        assert_eq!(source.length().unwrap_err().kind, "UnsafeRecovery");
        assert_eq!(source.seek(0).unwrap_err().kind, "UnsafeRecovery");
        assert_eq!(source.truncate(0).unwrap_err().kind, "UnsafeRecovery");
        assert_eq!(external.metadata()?.len(), 40);
        drop(source);
        let reopened = Source::new(&mut file, |_file: &File| Ok(()))?;
        assert_eq!(reopened.length()?, 40);
        Ok(())
    }

    #[test]
    fn empty_reads_past_eof_preserve_the_cursor_in_both_modes() -> Result<()> {
        for buffered in [true, false] {
            let mut file = tempfile::tempfile()?;
            file.write_all(b"snapshot")?;
            let mut source = Source::new(&mut file, |_file: &File| Ok(()))?;
            if !buffered {
                source.discard_buffer();
            }
            source.seek(32)?;
            assert!(source.read(0)?.is_empty());
            assert_eq!(source.length()?, 8);
            drop(source);
            assert_eq!(file.stream_position()?, 32);
            assert_eq!(file.metadata()?.len(), 8);
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn failed_seek_restores_the_handle_before_returning_an_error() -> Result<()> {
        use std::os::fd::OwnedFd;
        use std::os::unix::net::UnixStream;
        let (socket, _peer) = UnixStream::pair()?;
        let descriptor: OwnedFd = socket.into();
        let mut file = File::from(descriptor);
        let mut source = Source::new(&mut file, |_file: &File| Ok(()))?;
        assert!(source.seek(0).is_err());
        assert_eq!(source.length().unwrap_err().kind, "UnsafeRecovery");
        // A missing descriptor here would panic; a failed seek must restore it.
        source.discard_buffer();
        assert_eq!(source.read(0).unwrap_err().kind, "UnsafeRecovery");
        Ok(())
    }

    #[test]
    fn caught_barrier_unwind_keeps_the_source_unavailable() -> Result<()> {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let mut file = tempfile::tempfile()?;
        file.write_all(b"snapshot")?;
        let mut source = Source::new(&mut file, |file: &File| {
            file.set_len(0)?;
            panic!("injected barrier unwind after a file effect");
        })?;
        source.discard_buffer();
        assert!(catch_unwind(AssertUnwindSafe(|| source.sync())).is_err());
        assert_eq!(source.length().unwrap_err().kind, "UnsafeRecovery");
        assert_eq!(source.read(0).unwrap_err().kind, "UnsafeRecovery");
        assert_eq!(source.seek(0).unwrap_err().kind, "UnsafeRecovery");
        drop(source);
        assert_eq!(file.metadata()?.len(), 0);
        Ok(())
    }
}
