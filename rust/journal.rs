#[cfg(test)]
use crate::framing::Kind;
use crate::store::{mkdir, sync, sync_dir};
use crate::{Error, Fault, Result};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
#[cfg(test)]
use std::fs::File;
use std::path::PathBuf;
/// A journal keeps its descriptor, location and replay state together.
/// Read-only dereferencing preserves inspection of sequence and transactions.
/// Mutating those fields cannot authorize append after an uncertain outcome.
///
/// ```compile_fail
/// # use straw::journal::Journal;
/// fn clear_quarantine(journal: &mut Journal) {
///     journal.poisoned = false;
/// }
/// ```
/// ```compile_fail
/// # use straw::journal::Journal;
/// fn rewrite_history(journal: &mut Journal) {
///     journal.transactions.clear();
/// }
/// ```
/// ```compile_fail
/// # use straw::journal::Journal;
/// fn replace_path(journal: &mut Journal) {
///     journal.path = std::path::PathBuf::new();
/// }
/// ```
pub type Journal = crate::journal_owner::Journal<PathBuf>;
impl std::ops::Deref for Journal {
    type Target = crate::journal_state::State;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}
fn append_error(error: crate::journal_append::Error<Error>) -> Error {
    let error = match error {
        crate::journal_append::Error::Unavailable
        | crate::journal_append::Error::Write(crate::journal_state::AppendFailure::Unavailable) => {
            Error::new(
                "UnsafeRecovery",
                "Journal unavailable; stop owner and recover",
            )
        }
        crate::journal_append::Error::Events => {
            Error::new("ValueError", "Invalid journal event list")
        }
        crate::journal_append::Error::Json(error) => error.into(),
        crate::journal_append::Error::Write(crate::journal_state::AppendFailure::Write(error)) => {
            crate::wal_file::append_error(error)
        }
    };
    if ["StorageUnavailable", "QuotaExceeded"].contains(&error.kind.as_str()) {
        Error::new(
            "IndeterminateCommit",
            "Journal write/sync outcome unknown; recover and query original ID",
        )
    } else {
        error
    }
}

pub(crate) fn retirement_error(error: crate::queue_retirement::Error<Error>) -> Error {
    match error {
        crate::queue_retirement::Error::Append(error) => append_error(error),
        crate::queue_retirement::Error::Before(error)
        | crate::queue_retirement::Error::After(error)
        | crate::queue_retirement::Error::Consume(error) => error,
        crate::queue_retirement::Error::Apply(crate::queue_metadata::Error::Event) => {
            Error::new("InvalidReference", "Invalid queue metadata event")
        }
        crate::queue_retirement::Error::Apply(crate::queue_metadata::Error::State) => Error::new(
            "UnsafeRecovery",
            "Queue metadata path is missing or invalid",
        ),
    }
}

struct NativeIo<'a, 'f>(&'a mut Fault<'f>);
impl crate::journal_owner::Io<PathBuf, crate::wal_file::NativeLibrary> for NativeIo<'_, '_> {
    fn mkdir(&mut self, path: &PathBuf) -> Result<()> {
        mkdir(path.parent().unwrap(), self.0)
    }
    fn open(
        &mut self,
        path: &PathBuf,
        recover: bool,
        read_only: bool,
    ) -> Result<crate::journal_owner::Opened> {
        Ok(crate::journal_primitives::open(path, recover, read_only)?)
    }
    fn sync_new(&mut self, opened: &mut crate::journal_owner::Opened) -> Result<()> {
        sync(&opened.file, self.0)
    }
    fn sync_parent(
        &mut self,
        path: &PathBuf,
        _opened: &mut crate::journal_owner::Opened,
    ) -> Result<()> {
        sync_dir(path.parent().unwrap(), self.0)
    }
}
impl crate::wal_sink::Hooks<crate::wal_file::NativeFile<'_>> for NativeIo<'_, '_> {
    fn point(&mut self, _file: &mut crate::wal_file::NativeFile<'_>, point: &str) -> Result<()> {
        (self.0)(point)
    }
}
impl crate::journal_owner::RecoverySync<crate::wal_file::NativeFile<'_>> for NativeIo<'_, '_> {
    fn sync(&mut self, file: &mut crate::wal_file::NativeFile<'_>) -> Result<()> {
        sync(file.file(), self.0)
    }
}
pub(crate) fn owner_error(error: crate::journal_owner::Error<Error>) -> Error {
    use crate::journal_owner::Error as E;
    match error {
        E::Mode => Error::new("ValueError", "Read-only journal requires recovery"),
        E::Closed => Error::new(
            "UnsafeRecovery",
            "Journal unavailable; stop owner and recover",
        ),
        E::Identity => Error::new(
            "UnsafeRecovery",
            "Journal run/configuration identity mismatch",
        ),
        E::Io(error) => error,
        E::Append(error) => append_error(error),
        E::Recover(error) => {
            use crate::wal_recovery::Failure;
            crate::wal_file::error(match error {
                Failure::Device(error) => Failure::Device(error),
                Failure::Malformed(error) => Failure::Malformed(error),
                Failure::Apply(crate::journal_recovery::Error::Json(error)) => {
                    Failure::Apply(error.into())
                }
                Failure::Apply(crate::journal_recovery::Error::Events) => {
                    Failure::Apply(Error::new("CorruptData", "Invalid journal event list"))
                }
            })
        }
    }
}

impl Journal {
    /// Location of this journal. Its descriptor and replay state stay bound
    /// until the journal is closed; callers cannot replace them through a view.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub(crate) fn state_mut(&mut self) -> &mut crate::journal_state::State {
        &mut self.state
    }

    pub fn open(
        root: &std::path::Path,
        identity: &Value,
        recover: bool,
        read_only: bool,
        fault: &mut Fault<'_>,
    ) -> Result<Self> {
        crate::journal_owner::open::<_, crate::wal_file::NativeLibrary, _>(
            root.join("control/journal.log"),
            identity,
            recover,
            read_only,
            NativeIo(fault),
        )
        .map_err(owner_error)
    }
    #[cfg(test)]
    fn recover(&mut self, fault: &mut Fault<'_>) -> Result<()> {
        self.recover_with::<crate::wal_file::NativeLibrary, _>(crate::wal_file::NativeBarrier(
            |file: &File| sync(file, fault),
        ))
        .map_err(owner_error)?;
        Ok(())
    }
    pub fn append(&mut self, events: &Value, fault: &mut Fault<'_>) -> Result<u64> {
        self.append_with::<crate::wal_file::NativeLibrary, _>(
            events,
            crate::wal_file::NativeHooks(fault),
        )
        .map_err(owner_error)
    }

    #[cfg(test)]
    pub(crate) fn retire_roots(
        &mut self,
        business: &mut Value,
        roots: crate::queue_roots::Roots,
        fault: &mut Fault<'_>,
    ) -> Result<crate::queue_gc::Retired> {
        let file = self.file.as_mut().ok_or_else(|| {
            Error::new(
                "UnsafeRecovery",
                "Journal unavailable; stop owner and recover",
            )
        })?;
        crate::journal_io::retire::<crate::wal_file::NativeLibrary, _>(
            file,
            &mut self.state,
            business,
            roots,
            crate::wal_file::NativeHooks(fault),
        )
        .map_err(retirement_error)
    }

    pub(crate) fn prepare_gc<'p, D, H>(
        &mut self,
        business: &mut Value,
        cache: &mut crate::queue_progress::Cache,
        digester: &mut D,
        prefix: &'p str,
        hooks: H,
    ) -> std::result::Result<
        crate::queue_gc::Prepared<'p>,
        crate::queue_gc_prepare::Error<D::Error, Error>,
    >
    where
        D: crate::closure_node::Digest,
        H: crate::queue_gc_prepare::Preflight<Error> + crate::queue_progress::Reader<Error = Error>,
        for<'a> H: crate::wal_sink::Hooks<crate::wal_file::NativeFile<'a>>,
    {
        crate::queue_gc_prepare::for_queue::<crate::wal_file::NativeLibrary, D, H>(
            &mut self.file,
            &mut self.state,
            business,
            cache,
            digester,
            prefix,
            hooks,
        )
    }

    pub fn size(&self) -> u64 {
        self.path.metadata().map(|m| m.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::{Seek, SeekFrom, Write};
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn retirement_state() -> Value {
        let mut state = crate::queue_history::initial();
        state["consumers"]["training"] = json!({"processed_cursor":3});
        state["producers"]["sampler"] =
            json!({"continuation":{"manifest":{"segment":{"path":"raw/keep"}}}});
        state
    }
    fn retirement_roots(state: &Value) -> Result<crate::queue_roots::Roots> {
        struct Digest;
        impl crate::closure_node::Digest for Digest {
            type Error = Error;
            fn digest(&mut self, value: &Value) -> Result<String> {
                crate::digest(value)
            }
        }
        crate::queue_roots::collect(
            &mut Digest,
            state,
            &json!({"processed_positions":[4,7],"finished_batches":[]}),
        )
        .map_err(|_| Error::new("InvalidReference", "invalid retention test state"))
    }

    #[test]
    fn owner_admission_preserves_run_mode_and_closed_quarantine() -> Result<()> {
        let root = tempfile::tempdir()?;
        let identity = json!({"run":"owner-admission","config":{"revision":7}});
        let invalid = Journal::open(root.path(), &identity, false, true, &mut |_| Ok(()));
        assert_eq!(invalid.err().unwrap().kind, "ValueError");
        assert!(!root.path().join("control").exists());
        let mut writer = Journal::open(root.path(), &identity, false, false, &mut |_| Ok(()))?;
        assert_eq!(writer.sequence, 1);
        let path = writer.path.clone();
        let original = std::fs::read(&path)?;
        assert!(Journal::open(root.path(), &identity, false, false, &mut |_| Ok(())).is_err());
        assert_eq!(std::fs::read(&path)?, original);
        writer.close();
        writer.close();
        assert!(writer.closed());
        assert_eq!(writer.sequence, 1);
        assert_eq!(
            writer
                .append(&json!([{"type":"Test"}]), &mut |_| panic!(
                    "closed owner ran a hook"
                ))
                .unwrap_err()
                .kind,
            "UnsafeRecovery"
        );
        let wrong = json!({"run":"owner-admission","config":{"revision":8}});
        assert_eq!(
            Journal::open(root.path(), &wrong, true, false, &mut |_| Ok(()))
                .err()
                .unwrap()
                .kind,
            "UnsafeRecovery"
        );
        let mut reader = Journal::open(root.path(), &identity, true, true, &mut |_| {
            panic!("read-only recovery ran a barrier")
        })?;
        assert_eq!(
            reader.transactions,
            vec![json!([{"type":"Run","identity":identity}])]
        );
        assert_eq!(
            reader
                .append(&json!([{"type":"Test"}]), &mut |_| panic!(
                    "read-only owner ran a hook"
                ))
                .unwrap_err()
                .kind,
            "UnsafeRecovery"
        );
        assert_eq!(std::fs::read(&path)?, original);
        Ok(())
    }

    #[test]
    fn initialization_barriers_and_lost_run_reply_recover_conservatively() -> Result<()> {
        for point in [
            "before_file_sync",
            "before_directory_sync",
            "after_journal_sync",
        ] {
            let root = tempfile::tempdir()?;
            let identity = json!({"run":"owner-initialization"});
            let path = root.path().join("control/journal.log");
            let mut injected = false;
            let failed = Journal::open(root.path(), &identity, false, false, &mut |at| {
                if at == point && path.exists() {
                    injected = true;
                    return Err(Error::new(
                        "StorageUnavailable",
                        "injected initialization failure",
                    ));
                }
                Ok(())
            });
            assert!(injected);
            assert!(failed.is_err());
            let recovered = Journal::open(root.path(), &identity, true, false, &mut |_| Ok(()));
            if point == "after_journal_sync" {
                let recovered = recovered?;
                assert_eq!(recovered.sequence, 1);
                assert_eq!(
                    recovered.transactions,
                    vec![json!([{"type":"Run","identity":identity}])]
                );
            } else {
                assert_eq!(std::fs::metadata(&path)?.len(), 0);
                assert_eq!(recovered.err().unwrap().kind, "UnsafeRecovery");
            }
        }
        Ok(())
    }

    #[test]
    fn retention_wal_and_application_preserve_root_fields() -> Result<()> {
        let root = tempfile::tempdir()?;
        let identity = json!({"run":"retention"});
        let mut journal = Journal::open(root.path(), &identity, false, false, &mut |_| Ok(()))?;
        let mut state = retirement_state();
        let before = state.clone();
        let roots = retirement_roots(&state)?;
        journal.retire_roots(&mut state, roots, &mut |_| Ok(()))?;
        assert!(!journal.poisoned);
        assert_eq!(state["retired_prefix"], 3);
        let positions = state["retired_positions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_u64().unwrap())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(positions, std::collections::HashSet::from([4, 7]));
        for key in [
            "tasks",
            "commits",
            "batches",
            "readers",
            "consumers",
            "producers",
            "checkpoints",
        ] {
            assert_eq!(state[key], before[key]);
        }
        let reopened = Journal::open(root.path(), &identity, true, false, &mut |_| Ok(()))?;
        assert_eq!(reopened.sequence, 2);
        let event = &reopened.transactions[1][0];
        assert_eq!(event["type"], "RetentionAdvanced");
        assert_eq!(event["prefix"], 3);
        assert_eq!(event["positions"], state["retired_positions"]);
        Ok(())
    }

    #[test]
    fn retention_faults_never_return_a_token_and_recovery_replays_durable_metadata() -> Result<()> {
        for failure in [
            "before_journal_write",
            "after_journal_part",
            "before_journal_sync",
            "before_commit_apply",
            "after_commit_apply",
            "before_staging_consume",
        ] {
            let root = tempfile::tempdir()?;
            let identity = json!({"run":"retention-fault"});
            let mut journal = Journal::open(root.path(), &identity, false, false, &mut |_| Ok(()))?;
            let mut state = retirement_state();
            let before = state.clone();
            let roots = retirement_roots(&state)?;
            let outcome = journal.retire_roots(&mut state, roots, &mut |at| {
                if at == failure {
                    return Err(Error::new(
                        "StorageUnavailable",
                        "injected retirement fault",
                    ));
                }
                Ok(())
            });
            assert!(outcome.is_err(), "{failure}");
            assert_eq!(
                journal.poisoned,
                failure != "before_staging_consume",
                "{failure}"
            );
            for key in [
                "tasks",
                "commits",
                "batches",
                "readers",
                "consumers",
                "producers",
                "checkpoints",
            ] {
                assert_eq!(state[key], before[key], "{failure}: {key}");
            }
            let mut reopened = Journal::open(root.path(), &identity, true, false, &mut |_| Ok(()))?;
            let mut restored = retirement_state();
            for transaction in reopened.transactions.iter().skip(1) {
                for event in transaction.as_array().unwrap() {
                    crate::queue_metadata::apply(&mut restored, event).unwrap();
                }
            }
            assert_eq!(restored["producers"], before["producers"]);
            let roots = retirement_roots(&restored)?;
            reopened.retire_roots(&mut restored, roots, &mut |_| Ok(()))?;
            assert_eq!(restored["retired_prefix"], 3);
            assert!(!reopened.poisoned);
        }
        Ok(())
    }

    #[test]
    fn retention_application_unwind_keeps_journal_quarantined() -> Result<()> {
        for failure in ["before_commit_apply", "after_commit_apply"] {
            let root = tempfile::tempdir()?;
            let identity = json!({"run":"retention-unwind"});
            let mut journal = Journal::open(root.path(), &identity, false, false, &mut |_| Ok(()))?;
            let mut state = retirement_state();
            let roots = retirement_roots(&state)?;
            assert!(
                catch_unwind(AssertUnwindSafe(|| journal.retire_roots(
                    &mut state,
                    roots,
                    &mut |at| {
                        assert_ne!(at, failure, "injected retention unwind");
                        Ok(())
                    }
                )))
                .is_err()
            );
            assert!(journal.poisoned);
            assert_eq!(
                journal
                    .append(&json!([{"type":"Test"}]), &mut |_| Ok(()))
                    .unwrap_err()
                    .kind,
                "UnsafeRecovery"
            );
            let reopened = Journal::open(root.path(), &identity, true, false, &mut |_| Ok(()))?;
            assert_eq!(reopened.transactions[1][0]["type"], "RetentionAdvanced");
        }
        Ok(())
    }

    #[test]
    fn failed_replay_restarts_transactions_and_keeps_append_quarantined() -> Result<()> {
        for failure in ["json", "events", "sync"] {
            let root = tempfile::tempdir()?;
            let identity = json!({"run":"replay-retry"});
            let mut journal = Journal::open(root.path(), &identity, false, false, &mut |_| Ok(()))?;
            journal.append(&json!([{"type":"Test","request":"first"}]), &mut |_| Ok(()))?;
            let good_prefix = std::fs::read(&journal.path)?;
            journal.append(
                &json!([{"type":"Test","request":"second"}]),
                &mut |_| Ok(()),
            )?;
            let good = std::fs::read(&journal.path)?;
            let before = (journal.sequence, journal.offset);
            if failure != "sync" {
                let payload: &[u8] = if failure == "json" { b"{" } else { b"[]" };
                let prefix = crate::framing::prefix(Kind::Queue, 2, payload.len() as u64);
                let header = crate::framing::header(&prefix, &Sha256::digest(&prefix));
                let mut digest = Sha256::new();
                digest.update(&header);
                digest.update(payload);
                let trailer = crate::framing::trailer(Kind::Queue, &digest.finalize());
                let file = journal.file.as_mut().unwrap();
                file.seek(SeekFrom::Start(0))?;
                file.write_all(&good_prefix)?;
                file.write_all(&header)?;
                file.write_all(payload)?;
                file.write_all(&trailer)?;
                file.set_len(
                    (good_prefix.len() + header.len() + payload.len() + trailer.len()) as u64,
                )?;
                file.sync_all()?;
            }
            let failed = journal.recover(&mut |at| {
                if failure == "sync" && at == "before_file_sync" {
                    return Err(Error::new(
                        "StorageUnavailable",
                        "injected replay barrier failure",
                    ));
                }
                Ok(())
            });
            assert!(failed.is_err());
            assert!(journal.poisoned);
            assert_eq!((journal.sequence, journal.offset), before);
            assert_eq!(
                journal.transactions.len(),
                if failure == "sync" { 3 } else { 2 }
            );
            let failed_bytes = std::fs::read(&journal.path)?;
            assert_eq!(
                journal
                    .append(&json!([{"type":"Test"}]), &mut |_| Ok(()))
                    .unwrap_err()
                    .kind,
                "UnsafeRecovery"
            );
            assert_eq!(std::fs::read(&journal.path)?, failed_bytes);
            let file = journal.file.as_mut().unwrap();
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&good)?;
            file.set_len(good.len() as u64)?;
            file.sync_all()?;
            journal.recover(&mut |_| Ok(()))?;
            assert!(!journal.poisoned);
            assert_eq!(journal.transactions.len(), 3);
            assert_eq!(journal.transactions[1][0]["request"], "first");
            assert_eq!(journal.transactions[2][0]["request"], "second");
            assert_eq!(journal.sequence, 3);
            assert_eq!(journal.offset, good.len() as u64);
            assert_eq!(
                journal.append(&json!([{"type":"Test","request":"later"}]), &mut |_| Ok(()))?,
                3
            );
            journal.recover(&mut |_| Ok(()))?;
            assert_eq!(journal.transactions.len(), 4);
            assert_eq!(journal.transactions.last().unwrap()[0]["request"], "later");
        }
        Ok(())
    }

    #[test]
    fn caught_append_panic_requires_recovery_before_another_write() -> Result<()> {
        for point in [
            "before_journal_write",
            "after_journal_part",
            "after_journal_sync",
        ] {
            let root = tempfile::tempdir()?;
            let identity = json!({"run":"panic-recovery"});
            let mut journal = Journal::open(root.path(), &identity, false, false, &mut |_| Ok(()))?;
            let events = json!([{"type":"Test","request":"first"}]);
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                journal.append(&events, &mut |at| {
                    if at == point {
                        panic!("injected unwind at {point}");
                    }
                    Ok(())
                })
            }));
            assert!(outcome.is_err());
            assert!(
                journal.poisoned,
                "unwinding bypassed the append uncertainty guard"
            );
            let before = std::fs::read(&journal.path)?;
            assert_eq!(
                journal.append(&events, &mut |_| Ok(())).unwrap_err().kind,
                "UnsafeRecovery"
            );
            assert_eq!(std::fs::read(&journal.path)?, before);
            journal.close();
            let mut recovered =
                Journal::open(root.path(), &identity, true, false, &mut |_| Ok(()))?;
            assert_eq!(
                recovered.transactions.len(),
                if point == "after_journal_sync" { 2 } else { 1 }
            );
            recovered.append(&json!([{"type":"Test","request":"later"}]), &mut |_| Ok(()))?;
            let sequence = recovered.sequence;
            recovered.close();
            let reopened = Journal::open(root.path(), &identity, true, false, &mut |_| Ok(()))?;
            assert_eq!(reopened.sequence, sequence);
            assert_eq!(reopened.transactions.last().unwrap()[0]["request"], "later");
        }
        Ok(())
    }
}
