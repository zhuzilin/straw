------------------------ MODULE EnvironmentCrossLog ------------------------
EXTENDS EnvironmentWal
\* ENV-CROSS-LOG: complete-frame projections of the WAL prefix contract.
\* Header/body/trailer are separate algorithm steps. At most one complete
\* uncertain frame follows each durable prefix; partial frames project to the
\* old state. Recovery must validate/truncate and sync before exposing it.
\* Queue and catalog choices are INDEPENDENT, never one atomic transaction.
\* Derived from the WAL prefix set, not an independent durability premise.
\* Equal logical states can collapse an idempotent complete frame to a stutter.
EnvCrossRecovered(durable, current) ==
    EnvWalProjectedStates(durable, current, IF durable = current THEN 0 ELSE 3)
EnvCrossCatalogRecovered(durable, current, initial) == EnvCrossRecovered(durable, current)
EnvCrossQueueRecovered(durable, current) == EnvCrossRecovered(durable, current)
EnvCrossCatalogSynced(current) == current
EnvCrossQueueSynced(current) == current
\* Successful unlink establishes current absence; only the parent barrier
\* establishes durable absence. A failed barrier may have the same effect.
EnvCrossDirectorySynced(durableNames, pack) == durableNames \ {pack}
\* Application lifetime is separate from filesystem recovery or scheduling.
\* A permanent holder has no enabled finish action; fair GC must preserve it.
EnvCrossMayFinish(state, actor, permanent) == state = "active" /\ actor \notin permanent
=============================================================================
