--------------------- MODULE EnvironmentPersistence ---------------------
EXTENDS Naturals
\* ENV-SYNC: a successful barrier preserves the preceding state. The caller
\* must order the write/unlink before that barrier. Failure is not rollback.
EnvSyncedState(current) == current
\* One parent-directory barrier covers all preceding changes to its members,
\* not changes in another directory or unlinks issued after the barrier.
EnvSyncedDirectory(current, durable, members) ==
    (durable \ members) \cup {p \in members : EnvSyncedState(p \in current)}
\* ENV-DELETE-FAILURE: retry may observe the volatile name or a crash-restored
\* durable name. There is no claim that unlink itself is durable.
EnvNamesAfterFailure(present, durablePresent) == {present, durablePresent}
\* ENV-GC-STAT: advisory presence metadata may be stale in either direction.
\* It cannot establish successful Unlink/NotFound's actual-absence contract.
EnvStatPresence == BOOLEAN
\* ENV-DIRECTORY: mkdir can create any prefix before failing. Names and their
\* persisted parent entries are distinct. Root/aliases/mount topology stay fixed.
EnvCreatedDepth(current, limit) == current..limit
EnvDirectoryEntrySynced(durable, child) == durable \cup {child}
EnvReachableDirectories(durable, limit) ==
    {i \in 1..limit : 1..i \subseteq durable}
\* ENV-WAL-RECOVERY: one complete uncertain frame may survive. Torn prefixes
\* and corrupt complete frames are exercised by the byte-level crash campaign,
\* not represented by this value-level operator.
EnvRecoveredFrames(durable, written, empty) ==
    IF written # empty THEN {durable, written} ELSE {durable}
\* ENV-PUBLICATION-FINALIZE: a failed catalog operation may have persisted
\* its Boolean effect or left the prior durable state unchanged.
EnvUncertainBoolean(previous) == {previous, TRUE}
=============================================================================
