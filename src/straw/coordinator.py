"""Typed Python interface to the Rust queue state machine."""

import dataclasses
import json
import resource
import time
from pathlib import Path

from ._native import NativeCoordinator
from .protocol import AcquireResult, Assignment, CommitPage, CommitReceipt, Lease, Limits, RecordSetRef, TaskSpec


def _encode(value):
    return json.dumps(value, default=lambda v: dataclasses.asdict(v), ensure_ascii=False, allow_nan=False)


class _JournalView:
    def __init__(self, owner, path):
        self.owner = owner
        self.path = Path(path)
        self.stream = self

    def __getattr__(self, key):
        return json.loads(self.owner._native.diagnostics())[key]

    def close(self):
        self.owner.close()


class Coordinator:
    def __init__(
        self,
        store,
        *,
        queue_id="rollout",
        namespace=False,
        exclusive_owner,
        recover=False,
        limits=None,
        lease_seconds=300,
        clock=time.monotonic,
    ):
        self.store, self.queue_id = store, queue_id
        self.limits = limits or Limits()
        self.lease_seconds, self.clock = lease_seconds, clock
        self._cpu_started = time.process_time()
        options = dict(
            queue_id=queue_id,
            namespace=namespace,
            exclusive_owner=exclusive_owner,
            recover=recover,
            limits=dataclasses.asdict(self.limits),
            lease_seconds=lease_seconds,
            profile=store.backend.profile,
        )
        self._native = NativeCoordinator(store._config, _encode(options), store._fault)
        diagnostics = json.loads(self._native.diagnostics())
        self.journal = _JournalView(self, diagnostics["path"])
        self.recovery_seconds = diagnostics["recovery_seconds"]

    def _call(self, method, values):
        values.pop("self", None)
        return json.loads(self._native.call(method, _encode(values), self.clock(), self.store._fault))

    def __getattr__(self, key):
        if key not in {
            "tasks",
            "requests",
            "submissions",
            "consumers",
            "batches",
            "checkpoints",
            "commits",
            "producers",
            "sealed",
            "epoch",
            "draining",
            "storage_failed",
            "counters",
        }:
            raise AttributeError(key)
        return self._call("state", {})[key]

    def _usage(self):
        return self._call("usage", {})

    def close(self):
        self._native.close()

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_value, traceback):
        self.close()

    def submit_tasks(self, request_id, tasks, *, producer_id=None, producer_state=None):
        result = self._call("submit_tasks", locals())
        return result

    def producer_state(self, producer_id):
        result = self._call("producer_state", locals())
        return result

    def acquire(self, worker_id, max_tasks=1, *, task_ids=None, task_prefix=None, control=False):
        result = self._call("acquire", locals())
        return AcquireResult(
            result["status"],
            tuple(Assignment(TaskSpec.from_dict(a["task"]), Lease(**a["lease"])) for a in result["assignments"]),
        )

    def save_task_progress_many(self, updates, *, request_id):
        """Persist a bounded batch of leased continuation references atomically."""
        return self._call("save_task_progress_many", locals())

    def release_tasks(self, leases, *, request_id):
        """Release existing inputs atomically without spending failure retries."""
        return self._call("release_tasks", locals())

    def release_task_reads(self, leases):
        """Acknowledge actual end of input reads, including cancelled/expired leases.

        Completion, explicit failure and yielding already acknowledge their own
        reads. Timeout/cancellation/recovery alone do not prove a reader stopped.
        """
        return self._call("release_task_reads", locals())

    def heartbeat(self, leases):
        result = self._call("heartbeat", locals())
        return result

    def complete_task(self, lease, *, submission_id, result_ref, result_digest):
        result = self._call("complete_task", locals())
        return CommitReceipt.from_dict(result) if result is not None else None

    def fail_task(self, lease, *, request_id, failure, retryable=True):
        result = self._call("fail_task", locals())
        return result

    def yield_task(self, lease, *, request_id, input_ref):
        result = self._call("yield_task", locals())
        return result

    def save_task_progress(self, lease, *, request_id, input_ref):
        result = self._call("save_task_progress", locals())
        return result

    def cancel_task(self, task_id, *, request_id):
        result = self._call("cancel_task", locals())
        return result

    def seal_input(self, request_id):
        result = self._call("seal_input", locals())
        return result

    def lookup_submission(self, submission_id):
        result = self._call("lookup_submission", locals())
        return CommitReceipt.from_dict(result) if result is not None else None

    def read_commits(self, cursor=0, limit=100):
        result = self._call("read_commits", locals())
        return CommitPage(
            tuple(CommitReceipt.from_dict(r) for r in result["commits"]), result["cursor"], result["end_of_input"]
        )

    def retire_worker(self, worker_id):
        """Fence and retire a worker after the host has confirmed it stopped."""
        result = self._call("retire_worker", locals())
        return result

    def outstanding_reads(self):
        """Return leases whose read lifetimes have not been explicitly ended.

        Revoking an execution lease does not prove its reader has stopped. A
        supervisor may release these only after confirming that fact externally.
        """
        return tuple(Lease(**entry["lease"]) for entry in self._call("state", {})["readers"].values())

    def task_status(self, task_id):
        result = self._call("task_status", locals())
        return result

    def open_consumer(self, consumer_id, *, exclusive_owner):
        result = self._call("open_consumer", locals())
        return result

    def close_consumer(self, consumer_id, token):
        result = self._call("close_consumer", locals())
        return result

    def save_consumer_state(
        self, consumer_id, *, token, request_id, state_ref, fetch_cursor, processed_cursor, progress_ref=None
    ):
        result = self._call("save_consumer_state", locals())
        return result

    def load_consumer_state(self, consumer_id):
        result = self._call("load_consumer_state", locals())
        return result

    def plan_batch(self, consumer_id, *, token, batch_id, input_positions, plan_ref):
        result = self._call("plan_batch", locals())
        return result

    def batch_ready(
        self, consumer_id, *, token, batch_id, ready_ref, state_ref, fetch_cursor, processed_cursor, progress_ref=None
    ):
        result = self._call("batch_ready", locals())
        return result

    def get_batch(self, batch_id):
        result = self._call("get_batch", locals())
        return result

    def register_checkpoint(self, checkpoint_id, checkpoint_ref):
        result = self._call("register_checkpoint", locals())
        return result

    def release_checkpoint(self, checkpoint_id):
        return self._call("release_checkpoint", locals())

    def collect_garbage(self):
        """Release acknowledged history, retaining current queues and checkpoints.

        The training consumer's processed cursor/sparse positions acknowledge
        raw results. Its finished_batches acknowledge training reads on all ranks.
        Other queues and explicit storage owners independently retain their roots.
        """
        return self._call("collect_garbage", {})

    def snapshot(self):
        result = self._call("snapshot", locals())
        return RecordSetRef.from_dict(result)

    def metrics(self):
        result = self._call("metrics", locals())
        result.update(
            process_cpu_seconds=time.process_time() - self._cpu_started,
            process_rss_high_water_kib=resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
        )
        return result

    def drain(self):
        result = self._call("drain", locals())
        return result
