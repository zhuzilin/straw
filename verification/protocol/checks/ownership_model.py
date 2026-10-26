"""Bounded explicit-state model checker, independent of the Rust implementation.

One immutable pack, two queue journals, one reader, one checkpoint, one crash,
and at most one checkpoint rewind per queue.
Every enabled action/interleaving is explored to a fixed point (no random seed).
This checks a protocol abstraction, not Rust, JuiceFS or unbounded liveness.
"""

import argparse
import json
import time
from collections import deque
from dataclasses import dataclass, replace


@dataclass(frozen=True, slots=True)
class State:
    publication: int = 0  # 0 absent, 1 open/write, 2 payload durable, 3 staged
    present: bool = False
    durable: bool = False
    staged: bool = False
    sealed: bool = False
    owners: int = 0  # queue A, queue B, reader, checkpoint
    queues: tuple = (0, 0)  # idle, retained, WAL written, durable, replied, retired
    wal: int = 0
    acknowledged: int = 0
    rewinds: int = 0  # at most one checkpoint restore per queue
    reader: int = 0  # idle, active, finished
    checkpoint: int = 0
    gc: int = 0  # idle, locked/prepared, tombstone durable, unlink complete
    tombstone: bool = False
    crashed: bool = False


def step(s, mutation):
    if s.publication == 0:
        yield "register open pack + write payload", replace(s, publication=1, present=True)
    if s.publication == 1:
        yield "payload fsync", replace(s, publication=2, durable=True)
    if s.publication == 2:
        yield "persist staging ownership", replace(s, publication=3, staged=True)
    if s.publication == 3 and not s.sealed:
        yield "seal pack", replace(s, sealed=True)
    if s.staged and s.wal:
        yield "consume staging after queue WAL commit", replace(s, staged=False)
    unlocked = s.gc != 1 or mutation == "gc_without_lock"
    for q in range(2):
        bit = 1 << q
        phase = s.queues[q]

        def advance(n, **kwargs):
            phases = list(s.queues)
            phases[q] = n
            return replace(s, queues=tuple(phases), **kwargs)

        if phase == 0 and s.publication == 3 and s.present and not s.tombstone and unlocked:
            if mutation == "wal_before_retain":
                yield f"q{q}: BUG WAL before retention", advance(2)
            else:
                yield f"q{q}: retain destination", advance(1, owners=s.owners | bit)
        if phase == 1:
            yield f"q{q}: write queue WAL", advance(2)
        if phase == 2:
            yield f"q{q}: fsync queue WAL", advance(3, wal=s.wal | bit)
        if phase == 3:
            yield f"q{q}: reply success", advance(4, acknowledged=s.acknowledged | bit)
        if phase in (3, 4) and unlocked:
            yield f"q{q}: acknowledge use and persist root retirement", advance(
                5, wal=s.wal & ~bit, owners=s.owners & ~bit, acknowledged=s.acknowledged & ~bit
            )
        if phase == 5 and not s.rewinds & bit and s.checkpoint == 1 and s.present and not s.tombstone and unlocked:
            if mutation == "rewind_without_retain":
                yield f"q{q}: BUG restore cursor without adopting root", advance(
                    3, wal=s.wal | bit, rewinds=s.rewinds | bit
                )
            else:
                yield f"q{q}: adopt checkpoint root before restoring cursor", advance(
                    1, owners=s.owners | bit, rewinds=s.rewinds | bit
                )
    if s.publication == 3 and s.present and not s.tombstone and unlocked:
        if s.reader == 0:
            yield "reader: persist pin then start read", replace(s, owners=s.owners | 4, reader=1)
        if s.checkpoint == 0:
            yield "checkpoint: persist retained root", replace(s, owners=s.owners | 8, checkpoint=1)
    if s.reader == 1 and unlocked:
        yield "reader: finish read then release pin", replace(s, reader=2, owners=s.owners & ~4)
        if mutation == "reader_timeout":
            yield "BUG release pin while read is still active", replace(s, owners=s.owners & ~4)
    if s.checkpoint == 1 and unlocked:
        yield "checkpoint explicitly released", replace(s, checkpoint=2, owners=s.owners & ~8)
    if s.sealed and not s.staged and s.owners == 0 and s.gc == 0:
        yield "GC lock and scan owners", replace(s, gc=1)
    if s.gc == 1:
        yield "persist tombstone", replace(s, gc=2, tombstone=True)
        if mutation == "unlink_before_tombstone":
            yield "BUG unlink before tombstone fsync", replace(s, gc=3, present=False)
    if s.gc == 2 and s.present:
        yield "unlink tombstoned pack", replace(s, gc=3, present=False)
    if not s.crashed:
        # Whole coordinator process loss: uncommitted requests may be absent.
        # The alternative in which the write survives is explored by WAL fsync
        # followed by this crash. Data already acknowledged must survive.
        phases = list(s.queues)
        owners = s.owners
        for q, phase in enumerate(phases):
            if phase in (1, 2):
                phases[q] = 5
                owners &= ~(1 << q)  # recovery prunes an uncommitted queue root
        yield "crash and recover durable prefix", replace(
            s,
            queues=tuple(phases),
            owners=owners,
            crashed=True,
            gc=0 if s.gc == 1 else s.gc,
        )


def violation(s):
    live_queues = sum(1 << q for q, phase in enumerate(s.queues) if phase in (3, 4))
    if live_queues & ~s.owners:
        return "durable queue reference lacks durable storage ownership"
    if s.reader == 1 and not s.owners & 4:
        return "active reader lost its ownership"
    if s.checkpoint == 1 and not s.owners & 8:
        return "retained checkpoint lost its ownership"
    if (s.owners or s.staged or live_queues) and (not s.present or not s.durable or s.tombstone):
        return "live reference points to absent, non-durable or tombstoned data"
    if s.publication and not s.present and not s.tombstone:
        return "physical deletion lacks a durable tombstone"
    for q, phase in enumerate(s.queues):
        if s.acknowledged & (1 << q) and phase != 5 and not s.wal & (1 << q):
            return "acknowledged queue commit was lost"
    return None


def check(mutation=None):
    started = time.monotonic()
    initial = State()
    todo = deque([initial])
    seen = {initial: (None, None)}
    edges = 0
    while todo:
        state = todo.popleft()
        problem = violation(state)
        if problem:
            trace = []
            at = state
            while seen[at][0] is not None:
                previous, action = seen[at]
                trace.append(action)
                at = previous
            return {
                "mutation": mutation,
                "passed": False,
                "violation": problem,
                "counterexample": trace[::-1],
                "states": len(seen),
                "transitions": edges,
            }
        for action, following in step(state, mutation):
            edges += 1
            if following not in seen:
                seen[following] = (state, action)
                todo.append(following)
    return {
        "mutation": mutation,
        "passed": True,
        "states": len(seen),
        "transitions": edges,
        "seconds": time.monotonic() - started,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output")
    args = parser.parse_args()
    correct = check()
    mutants = [
        check(m)
        for m in (
            "wal_before_retain",
            "gc_without_lock",
            "unlink_before_tombstone",
            "reader_timeout",
            "rewind_without_retain",
        )
    ]
    report = {"scope": __doc__, "protocol": correct, "negative_controls": mutants}
    text = json.dumps(report, indent=2)
    if args.output:
        from pathlib import Path

        Path(args.output).write_text(text + "\n")
    print(text)
    if not correct["passed"] or any(m["passed"] for m in mutants):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
