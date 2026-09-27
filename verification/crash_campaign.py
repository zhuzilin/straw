"""Native WAL recovery and process-crash campaign on a private temporary root.

Enumerates every byte prefix and every single-byte XOR in a complete new frame
for both native journals. Abrupt child exit probes queue handoff durability.
This models prefix/torn appends; it is not a VM or storage-service power cut.
"""

import argparse
import json
import multiprocessing
import os
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import straw
from straw import Record, SharedFilesystemStore, TaskSpec
from straw.backend import FilesystemBackend
from straw.coordinator import Coordinator
from straw.errors import CorruptData, UnsafeRecovery
from straw.journal import Journal
from straw.protocol import RecordSetRef


def make_store(root):
    return SharedFilesystemStore(root, "campaign", online_gc=True)


def prefixes(root, catalog):
    if catalog:
        s = make_store(root)
        ref = s.publish([Record("data", b"immutable")], submission_id="source")
        path = root / "storage.log"
        before = path.read_bytes()
        s.retain("reader", [ref])
        after = path.read_bytes()

        def recover(complete):
            reader = make_store(root)
            result = reader.collect_garbage()
            assert result["owners"] == (2 if complete else 1), result
            assert next(reader.read(ref)).payload == b"immutable"

    else:
        backend = FilesystemBackend(root)
        identity = {"run_id": "campaign"}
        j = Journal(backend, identity)
        path = j.path
        before = path.read_bytes()
        event = [{"type": "Example", "request": "idempotent-operation", "value": [1, 2, 3]}]
        j.append(event)
        j.close()
        after = path.read_bytes()

        def recover(complete):
            j = Journal(backend, identity, recover=True, read_only=True)
            try:
                assert len(j.transactions) == (2 if complete else 1)
                if complete:
                    assert j.transactions[-1] == event
            finally:
                j.close()

    delta = len(after) - len(before)
    for n in range(delta + 1):
        path.write_bytes(after[: len(before) + n])
        recover(n == delta)
    for offset in range(len(before), len(after)):
        damaged = bytearray(after)
        damaged[offset] ^= 0x80
        path.write_bytes(damaged)
        try:
            recover(True)
        except (CorruptData, UnsafeRecovery):
            assert path.read_bytes() == damaged, "Corruption must not be truncated as an uncommitted tail"
        else:
            raise AssertionError(f"Undetected corruption at frame byte {offset - len(before)}")
    path.write_bytes(after)
    return {"prefixes": delta + 1, "corruptions": delta, "frame_bytes": delta}


def killed_transfer(root, reference, phase, occurrence, reply):
    s = make_store(root)
    q = Coordinator(
        s,
        queue_id="destination",
        namespace=True,
        exclusive_owner="previous owner exited",
        recover=True,
    )
    hits = 0

    def fault(at):
        nonlocal hits
        if at == phase:
            hits += 1
            if hits == occurrence:
                os._exit(70)  # no unwinding, destructors, close or Python cleanup

    s.backend.fault = fault
    q.submit_tasks("handoff", [TaskSpec("task", input_ref=RecordSetRef.from_dict(reference))])
    reply.send("acknowledged")
    os._exit(0)


def process_crashes(root, hosts=None):
    import dataclasses

    cases = [
        (phase, n)
        for phase, count in (
            ("before_storage_catalog_write", 1),
            ("after_storage_catalog_part", 3),
            ("before_storage_catalog_sync", 1),
            ("after_storage_catalog_sync", 1),
            ("before_journal_write", 1),
            ("after_journal_part", 3),
            ("before_journal_sync", 1),
            ("after_journal_sync", 1),
            ("no_failure", 1),
        )
        for n in range(1, count + 1)
    ]
    results = []
    context = multiprocessing.get_context("spawn")
    for i, (phase, occurrence) in enumerate(cases):
        directory = root / str(i)
        s = make_store(directory)
        ref = s.publish([Record("source", b"still live")], submission_id="source")
        source = Coordinator(s, queue_id="source", namespace=True, exclusive_owner="campaign")
        source.submit_tasks("source", [TaskSpec("source", input_ref=ref)])
        s.seal()
        target = Coordinator(
            make_store(directory),
            queue_id="destination",
            namespace=True,
            exclusive_owner="campaign",
        )
        target.close()
        if hosts:
            host = hosts[i % len(hosts)]
            command = [
                "env",
                f"PYTHONPATH={Path(__file__).resolve().parents[1] / 'src'}",
                "python",
                str(Path(__file__).resolve()),
                "--child-root",
                str(directory),
                "--child-ref",
                json.dumps(dataclasses.asdict(ref)),
                "--child-phase",
                phase,
                "--child-occurrence",
                str(occurrence),
            ]
            result = subprocess.run(
                ["ssh", "-o", "BatchMode=yes", host, shlex.join(command)],
                capture_output=True,
                text=True,
                timeout=70,
            )
            assert result.returncode == (0 if phase == "no_failure" else 70), (
                phase,
                result.stderr,
            )
            acknowledged = "ACK:acknowledged" in result.stdout.splitlines()
        else:
            host = "local-spawn"
            recv, send = context.Pipe(duplex=False)
            process = context.Process(
                target=killed_transfer,
                args=(str(directory), dataclasses.asdict(ref), phase, occurrence, send),
            )
            process.start()
            send.close()
            process.join(30)
            if process.is_alive():
                process.kill()
                process.join()
                raise TimeoutError(phase)
            assert process.exitcode == (0 if phase == "no_failure" else 70), (
                phase,
                process.exitcode,
            )
            acknowledged = False
            if recv.poll():
                try:
                    acknowledged = recv.recv() == "acknowledged"
                except EOFError:
                    pass
        target = Coordinator(
            make_store(directory),
            queue_id="destination",
            namespace=True,
            exclusive_owner="child has exited",
            recover=True,
        )
        accepted = bool(target.task_status("task"))
        if acknowledged:
            assert accepted, "Acknowledged handoff lost in recovery"
        # The source releases only after the child is dead/recovered. Destination
        # WAL either owns the input or has no task; neither case may dangle.
        source.cancel_task("source", request_id="done")
        source.collect_garbage()
        target.collect_garbage()
        if accepted:
            assert next(s.read(ref)).payload == b"still live"
        results.append(
            {
                "host": host,
                "phase": phase,
                "occurrence": occurrence,
                "acknowledged": acknowledged,
                "recovered_task": accepted,
            }
        )
        source.close()
        target.close()
    return results


def killed_rewind(root, phase, occurrence):
    s = make_store(root)
    q = Coordinator(s, exclusive_owner="previous owner exited", recover=True)
    token = q.open_consumer("training", exclusive_owner="replacement trainer")
    state = q.load_consumer_state("training")
    hits = 0

    def fault(at):
        nonlocal hits
        if at == phase:
            hits += 1
            if hits == occurrence:
                os._exit(70)

    s.backend.fault = fault
    q.save_consumer_state(
        "training",
        token=token,
        request_id="restore",
        state_ref=RecordSetRef.from_dict(state["state_ref"]),
        fetch_cursor=1,
        processed_cursor=0,
    )
    print("ACK:restored", flush=True)
    os._exit(0)


def rewind_crashes(root, hosts):
    cases = [
        (phase, n)
        for phase, count in (
            ("before_storage_catalog_write", 1),
            ("after_storage_catalog_part", 3),
            ("before_storage_catalog_sync", 1),
            ("after_storage_catalog_sync", 1),
            ("before_journal_write", 1),
            ("after_journal_part", 3),
            ("before_journal_sync", 1),
            ("after_journal_sync", 1),
            ("no_failure", 1),
        )
        for n in range(1, count + 1)
    ]
    results = []
    for i, (phase, occurrence) in enumerate(cases):
        directory = root / str(i)
        s = make_store(directory)
        q = Coordinator(s, exclusive_owner="campaign")
        q.submit_tasks("input", [TaskSpec("task")])
        lease = q.acquire("worker").assignments[0].lease
        ref = s.publish(
            [
                Record(
                    "result", b"retained history", metadata={"task_id": lease.task_id, "attempt_id": lease.attempt_id}
                )
            ],
            submission_id="result",
        )
        receipt = q.complete_task(lease, submission_id="accepted", result_ref=ref, result_digest=ref.digest)
        saved = s.publish([Record("saved", b"checkpoint")], submission_id="checkpoint", dependencies=[ref])
        s.retain("checkpoint", [saved])
        token = q.open_consumer("training", exclusive_owner="trainer")
        q.save_consumer_state(
            "training", token=token, request_id="advanced", state_ref=saved, fetch_cursor=1, processed_cursor=1
        )
        s.seal()
        q.collect_garbage()
        q.close()
        host = hosts[i % len(hosts)] if hosts else "local"
        command = [
            "env",
            f"PYTHONPATH={Path(__file__).resolve().parents[1] / 'src'}",
            "python",
            str(Path(__file__).resolve()),
            "--child-rewind",
            str(directory),
            "--child-phase",
            phase,
            "--child-occurrence",
            str(occurrence),
        ]
        if hosts:
            command = ["ssh", "-o", "BatchMode=yes", host, shlex.join(command)]
        else:
            # Exercise the same installed wheel/source as the parent. An
            # isolated wheel check intentionally has no `env` executable or
            # Rust toolchain on PATH, and must not import the source checkout.
            command = [sys.executable, *(["-I"] if sys.flags.isolated else []), *command[3:]]
        child = subprocess.run(
            command,
            capture_output=True,
            text=True,
            timeout=70,
            env={**os.environ, "PYTHONPATH": str(Path(straw.__file__).resolve().parents[1])},
        )
        assert child.returncode == (0 if phase == "no_failure" else 70), (phase, child.stderr)
        recovered = Coordinator(make_store(directory), exclusive_owner="child confirmed stopped", recover=True)
        committed = recovered.load_consumer_state("training")["processed_cursor"] == 0
        acknowledged = "ACK:restored" in child.stdout.splitlines()
        assert not acknowledged or committed
        recovered.collect_garbage()
        token = recovered.open_consumer("training", exclusive_owner="replacement")
        recovered.save_consumer_state(
            "training", token=token, request_id="retry-restore", state_ref=saved, fetch_cursor=1, processed_cursor=0
        )
        assert recovered.read_commits(0, 1).commits == (receipt,)
        s.release("checkpoint")
        recovered.collect_garbage()
        assert next(s.read(ref)).payload == b"retained history"
        recovered.close()
        results.append(
            dict(
                host=host,
                phase=phase,
                occurrence=occurrence,
                acknowledged=acknowledged,
                restored_after_crash=committed,
            )
        )
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output")
    parser.add_argument(
        "--hosts",
        nargs=4,
        help="Exercise child handoff crashes across four shared-mount clients",
    )
    parser.add_argument("--child-rewind")
    parser.add_argument("--child-root")
    parser.add_argument("--child-ref")
    parser.add_argument("--child-phase")
    parser.add_argument("--child-occurrence", type=int)
    parser.add_argument(
        "--parent",
        help="Optional parent for a private, automatically deleted temporary directory",
    )
    args = parser.parse_args()
    if args.child_rewind:
        signal.signal(signal.SIGALRM, lambda *_: os._exit(71))
        signal.alarm(60)
        killed_rewind(args.child_rewind, args.child_phase, args.child_occurrence)
        return
    if args.child_root:
        from types import SimpleNamespace

        signal.signal(signal.SIGALRM, lambda *_: os._exit(71))
        signal.alarm(60)
        killed_transfer(
            args.child_root,
            json.loads(args.child_ref),
            args.child_phase,
            args.child_occurrence,
            SimpleNamespace(send=lambda value: print(f"ACK:{value}", flush=True)),
        )
        return
    if not args.output:
        parser.error("--output is required for a campaign")
    started = time.monotonic()
    root = Path(tempfile.mkdtemp(prefix="straw-crash-campaign-", dir=args.parent))
    print(f"Campaign root (retained on failure): {root}", flush=True)
    report = {
        "queue_wal": prefixes(root / "queue", False),
        "storage_wal": prefixes(root / "catalog", True),
        "process_crashes": process_crashes(root / "processes", args.hosts),
        "rewind_crashes": rewind_crashes(root / "rewinds", args.hosts),
    }
    shutil.rmtree(root)
    report.update(
        seconds=time.monotonic() - started,
        temporary_root_removed=not root.exists(),
        scope="Prefix persistence and process exit, not power-loss or storage-service failure",
        passed=True,
    )
    Path(args.output).write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
